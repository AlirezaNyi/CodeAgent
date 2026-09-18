//! RAYA Agent CLI.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use raya_agent::Orchestrator;
use raya_core::{
    AgentTask, Config, LogFormat, TaskId, TaskPhase, ToolCallId, discover, init_tracing,
};
use raya_llm::{LlmProvider, MockProvider, OpenAiCompatibleProvider};
use raya_policy::PolicyEngine;
use raya_protocol::{AppState, serve};
use raya_store::Store;
use raya_tools::default_registry;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::info;

#[derive(Debug, Parser)]
#[command(
    name = "raya",
    version,
    about = "RAYA Agent — local coding agent runtime"
)]
struct Cli {
    /// Emit machine-readable JSON where applicable.
    #[arg(long, global = true)]
    json: bool,

    /// Override project root (default: discover from cwd).
    #[arg(long, global = true)]
    project: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Agent task operations.
    Agent {
        #[command(subcommand)]
        command: AgentCommands,
    },
    /// Start the local HTTP API server.
    Serve {
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        port: Option<u16>,
    },
}

#[derive(Debug, Subcommand)]
enum AgentCommands {
    /// Run a coding task.
    Run {
        /// Natural-language task description.
        task: String,
    },
    /// Show task status.
    Status { task_id: Option<String> },
    /// Show structured task events.
    Logs {
        task_id: String,
        #[arg(long)]
        follow: bool,
    },
    /// Cancel a running task.
    Cancel { task_id: String },
    /// Approve a pending tool call.
    Approve { task_id: String, call_id: String },
    /// Index the repository (file hash metadata).
    Index,
    /// Explain a file with bounded context.
    Explain { file: PathBuf },
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let project = discover(cli.project.as_deref()).context("project discovery failed")?;
    let mut config = Config::load(project.path()).context("failed to load config")?;
    init_tracing(LogFormat::parse(&config.server.log_format));

    let db_path = config.database_path(project.path());
    let store = Arc::new(Store::open(&db_path).context("open store")?);
    let policy = PolicyEngine::new(config.policy.clone());
    // For local agent.run with mock, allow shell auto to reduce friction in demos;
    // policy config still applies. Keep defaults from config.
    let tools = Arc::new(default_registry(policy));
    let llm = build_llm(&config)?;

    let project_rec = store.get_or_create_project(
        project.path(),
        project
            .path()
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("project"),
    )?;

    match cli.command {
        Commands::Agent { command } => match command {
            AgentCommands::Run { task } => {
                let agent_task = AgentTask::new(
                    project_rec.id,
                    task,
                    config.agent.max_iterations,
                    config.agent.max_tool_calls,
                    config.context.max_tokens.saturating_mul(4),
                    config.context.max_tokens,
                    Some(
                        chrono::Utc::now()
                            + chrono::Duration::seconds(config.agent.timeout_seconds as i64),
                    ),
                );
                let id = agent_task.id;
                store.create_task(&agent_task)?;

                let orch = Orchestrator::new(
                    store.clone(),
                    tools,
                    llm,
                    config.clone(),
                    project.path().to_path_buf(),
                );
                let cancel = CancellationToken::new();
                let ctrl = cancel.clone();
                tokio::spawn(async move {
                    tokio::signal::ctrl_c().await.ok();
                    ctrl.cancel();
                });

                info!(task_id = %id, "running task");
                let finished = orch.run(id, cancel).await.context("orchestrator")?;
                print_task(cli.json, &finished);
                Ok(exit_for_phase(finished.phase))
            }
            AgentCommands::Status { task_id } => {
                if let Some(id) = task_id {
                    let tid: TaskId = id.parse().context("task id")?;
                    let task = store
                        .get_task(tid)?
                        .with_context(|| format!("task {id} not found"))?;
                    print_task(cli.json, &task);
                } else {
                    let tasks = store.list_tasks(Some(project_rec.id), 20)?;
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&tasks)?);
                    } else {
                        for t in tasks {
                            println!("{}  {:?}  {}", t.id, t.phase, truncate(&t.request, 60));
                        }
                    }
                }
                Ok(ExitCode::SUCCESS)
            }
            AgentCommands::Logs { task_id, follow } => {
                let tid: TaskId = task_id.parse().context("task id")?;
                let mut after = 0i64;
                loop {
                    let events = store.list_events(tid, Some(after), 100)?;
                    for (seq, e) in &events {
                        after = *seq;
                        if cli.json {
                            println!(
                                "{}",
                                serde_json::json!({
                                    "seq": seq,
                                    "kind": e.kind.as_str(),
                                    "payload": e.payload,
                                    "created_at": e.created_at,
                                })
                            );
                        } else {
                            println!(
                                "[{}] {} {}",
                                e.created_at.to_rfc3339(),
                                e.kind.as_str(),
                                e.payload
                            );
                        }
                    }
                    if !follow {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                    if let Some(t) = store.get_task(tid)?
                        && t.phase.is_terminal()
                        && events.is_empty()
                    {
                        break;
                    }
                }
                Ok(ExitCode::SUCCESS)
            }
            AgentCommands::Cancel { task_id } => {
                let tid: TaskId = task_id.parse().context("task id")?;
                let ok = store.request_cancel(tid)?;
                if cli.json {
                    println!("{}", serde_json::json!({"cancelled": ok, "id": task_id}));
                } else {
                    println!(
                        "{}",
                        if ok {
                            "cancel requested"
                        } else {
                            "task not found"
                        }
                    );
                }
                Ok(if ok {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                })
            }
            AgentCommands::Approve { task_id, call_id } => {
                let tid: TaskId = task_id.parse().context("task id")?;
                let cid: ToolCallId = call_id.parse().context("call id")?;
                store.set_approval(tid, cid, true)?;
                if cli.json {
                    println!(
                        "{}",
                        serde_json::json!({"approved": true, "task_id": task_id, "call_id": call_id})
                    );
                } else {
                    println!("approved {call_id} for task {task_id}");
                }
                Ok(ExitCode::SUCCESS)
            }
            AgentCommands::Index => {
                let mut count = 0u64;
                let walker = ignore::WalkBuilder::new(project.path())
                    .git_ignore(true)
                    .build();
                for entry in walker.flatten() {
                    if entry.file_type().is_some_and(|t| t.is_file()) {
                        let meta = entry.metadata().ok();
                        let len = meta.map(|m| m.len()).unwrap_or(0);
                        store.set_index_meta(
                            &format!("file:{}", entry.path().display()),
                            &len.to_string(),
                        )?;
                        count += 1;
                    }
                }
                store.set_index_meta("last_index_count", &count.to_string())?;
                if cli.json {
                    println!("{}", serde_json::json!({"indexed_files": count}));
                } else {
                    println!("indexed {count} files");
                }
                Ok(ExitCode::SUCCESS)
            }
            AgentCommands::Explain { file } => {
                let engine = raya_context::ContextEngine::new(
                    config.context.max_tokens,
                    config.context.max_files,
                );
                let request = format!("Explain the file {}", file.display());
                let bundle = engine.build(project.path(), &request)?;
                let context = engine.render_prompt(&bundle);
                let resp = llm
                    .complete(raya_core::CompletionRequest {
                        model: config.llm.model.clone(),
                        messages: vec![
                            raya_core::Message::system(
                                "Explain the following code clearly and briefly.",
                            ),
                            raya_core::Message::user(format!(
                                "File: {}\n\nContext:\n{context}",
                                file.display()
                            )),
                        ],
                        tools: None,
                        structured_json: false,
                        temperature: Some(0.2),
                        max_tokens: Some(2048),
                    })
                    .await?;
                if cli.json {
                    println!(
                        "{}",
                        serde_json::json!({"explanation": resp.content, "files": bundle.snippets.len()})
                    );
                } else {
                    println!("{}", resp.content);
                }
                Ok(ExitCode::SUCCESS)
            }
        },
        Commands::Serve { host, port } => {
            if let Some(h) = host {
                config.server.host = h;
            }
            if let Some(p) = port {
                config.server.port = p;
            }
            let state = AppState {
                store,
                tools,
                llm,
                config: config.clone(),
                project_root: project.path().to_path_buf(),
                started: Instant::now(),
                task_slots: Arc::new(Semaphore::new(
                    config.resources.max_parallel_agents as usize,
                )),
                cancel_tokens: Arc::new(tokio::sync::Mutex::new(Default::default())),
            };
            serve(state).await.map_err(|e| anyhow::anyhow!(e))?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn build_llm(config: &Config) -> Result<Arc<dyn LlmProvider>> {
    match config.llm.provider.to_ascii_lowercase().as_str() {
        "mock" => Ok(Arc::new(MockProvider::default_script())),
        "openai" => {
            let provider = OpenAiCompatibleProvider::from_env(
                &config.llm.base_url,
                &config.llm.model,
                config.llm.timeout_seconds,
                CancellationToken::new(),
            )
            .map_err(|e| anyhow::anyhow!(e))?;
            Ok(Arc::new(provider))
        }
        other => bail!("unsupported llm.provider: {other}"),
    }
}

fn print_task(json: bool, task: &AgentTask) {
    if json {
        println!("{}", serde_json::to_string_pretty(task).unwrap_or_default());
    } else {
        println!("id:      {}", task.id);
        println!("phase:   {:?}", task.phase);
        println!("status:  {:?}", task.status());
        println!("request: {}", task.request);
        if let Some(err) = &task.error {
            println!("error:   {err}");
        }
        if let Some(step) = &task.current_step {
            println!("summary: {step}");
        }
    }
}

fn exit_for_phase(phase: TaskPhase) -> ExitCode {
    match phase {
        TaskPhase::Completed => ExitCode::SUCCESS,
        TaskPhase::WaitingApproval => ExitCode::from(2),
        _ => ExitCode::FAILURE,
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}
