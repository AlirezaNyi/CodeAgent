//! RAYA Agent CLI.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use raya_agent::Orchestrator;
use raya_core::{
    AgentTask, Config, LogFormat, TaskId, TaskPhase, ToolCallId, discover, init_tracing,
};
use raya_llm::{ModelRole, ModelRouter, OpenAiCompatibleProvider};
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
    /// Initialize `.raya/` config, rules, and AGENTS.md convention.
    Init,
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
    /// Index the repository (hashes, FTS5, symbols).
    Index,
    /// Recent file/tool activity across tasks.
    Activity {
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
    /// Export a human-readable receipt for a task.
    Receipt { task_id: String },
    /// Explain a file with bounded context.
    Explain { file: PathBuf },
    /// List or probe configured LLM lanes.
    Llm {
        #[command(subcommand)]
        command: LlmCommands,
    },
}

#[derive(Debug, Subcommand)]
enum LlmCommands {
    /// List configured LLM lanes (and the active selection).
    Lanes,
    /// Show role → lane/model routing table.
    Routes,
    /// Probe `GET {base_url}/models` on the active (or named) lane.
    Probe {
        /// Override lane name for this probe.
        #[arg(long)]
        lane: Option<String>,
    },
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

    if matches!(cli.command, Commands::Init) {
        let root = match &cli.project {
            Some(p) => p.clone(),
            None => std::env::current_dir()?,
        };
        return cmd_init(&root, cli.json);
    }

    let project = discover(cli.project.as_deref()).context("project discovery failed")?;
    let mut config = Config::load(project.path()).context("failed to load config")?;
    config
        .llm
        .apply_active_lane()
        .context("failed to resolve llm lane")?;
    init_tracing(LogFormat::parse(&config.server.log_format));

    let db_path = config.database_path(project.path());
    let store = Arc::new(Store::open(&db_path).context("open store")?);
    let policy = PolicyEngine::new(config.policy.clone());
    let tools = Arc::new(default_registry(policy));
    let router = build_router(&config)?;

    let project_rec = store.get_or_create_project(
        project.path(),
        project
            .path()
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("project"),
    )?;

    match cli.command {
        Commands::Init => unreachable!(),
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

                let orch = Orchestrator::with_router(
                    store.clone(),
                    tools,
                    router.clone(),
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
                                    "evidence": e.evidence.as_str(),
                                    "payload": e.payload,
                                    "created_at": e.created_at,
                                })
                            );
                        } else {
                            println!(
                                "[{}] {} [{}] {}",
                                e.created_at.to_rfc3339(),
                                e.kind.as_str(),
                                e.evidence.as_str(),
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
                let stats = raya_index::index_project(&store, project.path())
                    .map_err(|e| anyhow::anyhow!(e))?;
                if cli.json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "scanned": stats.scanned,
                            "updated": stats.updated,
                            "unchanged": stats.unchanged,
                            "removed": stats.removed,
                            "symbols": stats.symbols,
                        })
                    );
                } else {
                    println!(
                        "indexed: scanned={} updated={} unchanged={} removed={} symbols={}",
                        stats.scanned, stats.updated, stats.unchanged, stats.removed, stats.symbols
                    );
                }
                Ok(ExitCode::SUCCESS)
            }
            AgentCommands::Activity { limit } => {
                let tasks = store.list_tasks(Some(project_rec.id), limit.max(5))?;
                let mut rows = Vec::new();
                for t in &tasks {
                    let events = store.list_events(t.id, None, 200)?;
                    for (seq, e) in events {
                        if matches!(
                            e.kind,
                            raya_core::EventKind::FileModified
                                | raya_core::EventKind::ToolCompleted
                                | raya_core::EventKind::PhaseChanged
                                | raya_core::EventKind::ApprovalRequested
                        ) {
                            rows.push(serde_json::json!({
                                "task_id": t.id.to_string(),
                                "seq": seq,
                                "kind": e.kind.as_str(),
                                "evidence": e.evidence.as_str(),
                                "created_at": e.created_at,
                                "payload": e.payload,
                            }));
                        }
                    }
                }
                rows.sort_by(|a, b| {
                    let ta = a.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
                    let tb = b.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
                    tb.cmp(ta)
                });
                rows.truncate(limit as usize);
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&rows)?);
                } else {
                    for r in &rows {
                        println!(
                            "{}  {}  [{}]  {}",
                            r.get("created_at").and_then(|v| v.as_str()).unwrap_or(""),
                            r.get("kind").and_then(|v| v.as_str()).unwrap_or(""),
                            r.get("evidence").and_then(|v| v.as_str()).unwrap_or(""),
                            r.get("task_id").and_then(|v| v.as_str()).unwrap_or(""),
                        );
                    }
                }
                Ok(ExitCode::SUCCESS)
            }
            AgentCommands::Receipt { task_id } => {
                let tid: TaskId = task_id.parse().context("task id")?;
                let task = store
                    .get_task(tid)?
                    .with_context(|| format!("task {task_id} not found"))?;
                let events = store.list_events(tid, None, 1000)?;
                let md = render_receipt(&task, &events);
                if cli.json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "task_id": task.id.to_string(),
                            "phase": format!("{:?}", task.phase),
                            "markdown": md,
                            "events": events.len(),
                        })
                    );
                } else {
                    print!("{md}");
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
                let resp = router
                    .provider_for(ModelRole::Summary)
                    .complete(raya_core::CompletionRequest {
                        model: router.model_for(ModelRole::Summary),
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
            AgentCommands::Llm { command } => match command {
                LlmCommands::Lanes => cmd_llm_lanes(&config, cli.json),
                LlmCommands::Routes => cmd_llm_routes(&router, &config, cli.json),
                LlmCommands::Probe { lane } => {
                    cmd_llm_probe(&config, lane.as_deref(), cli.json).await
                }
            },
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
                router: router.clone(),
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

fn build_router(config: &Config) -> Result<Arc<ModelRouter>> {
    ModelRouter::from_config(config, CancellationToken::new())
        .map(Arc::new)
        .map_err(|e| anyhow::anyhow!(e))
}

fn cmd_llm_routes(router: &ModelRouter, config: &Config, json: bool) -> Result<ExitCode> {
    let rows: Vec<serde_json::Value> = router
        .routes()
        .into_iter()
        .map(|(role, lane, model)| {
            serde_json::json!({
                "role": role.as_str(),
                "lane": lane,
                "model": model,
            })
        })
        .collect();
    if json {
        println!(
            "{}",
            serde_json::json!({
                "provider": config.llm.provider,
                "routes": rows,
            })
        );
    } else {
        println!("provider={}", config.llm.provider);
        println!("role\tlane\tmodel");
        for row in &rows {
            let role = row.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let lane = row.get("lane").and_then(|v| v.as_str()).unwrap_or("");
            let model = row.get("model").and_then(|v| v.as_str()).unwrap_or("-");
            let model = if model.is_empty() { "-" } else { model };
            println!("{role}\t{lane}\t{model}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_llm_lanes(config: &Config, json: bool) -> Result<ExitCode> {
    let active = config.llm.resolve_lane().map_err(|e| anyhow::anyhow!(e))?;
    let rows: Vec<serde_json::Value> = if config.llm.lanes.is_empty() {
        vec![serde_json::json!({
            "name": active.name,
            "base_url": active.base_url,
            "model": active.model,
            "active": true,
            "legacy": true,
        })]
    } else {
        config
            .llm
            .lanes
            .iter()
            .map(|lane| {
                serde_json::json!({
                    "name": lane.name,
                    "base_url": lane.base_url,
                    "model": lane.model,
                    "api_key_env": lane.api_key_env,
                    "active": lane.name == active.name,
                })
            })
            .collect()
    };
    if json {
        println!(
            "{}",
            serde_json::json!({
                "provider": config.llm.provider,
                "active_lane": active.name,
                "lanes": rows,
            })
        );
    } else {
        println!(
            "provider={}  active_lane={}",
            config.llm.provider, active.name
        );
        for row in &rows {
            let name = row.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let base = row.get("base_url").and_then(|v| v.as_str()).unwrap_or("");
            let model = row.get("model").and_then(|v| v.as_str()).unwrap_or("");
            let marker = if row.get("active").and_then(|v| v.as_bool()).unwrap_or(false) {
                "*"
            } else {
                " "
            };
            println!("{marker} {name}\t{model}\t{base}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_llm_probe(config: &Config, lane: Option<&str>, json: bool) -> Result<ExitCode> {
    let mut cfg = config.clone();
    if let Some(name) = lane {
        cfg.llm.lane = name.to_string();
        cfg.llm
            .apply_active_lane()
            .context("failed to resolve llm lane")?;
    }
    let resolved = cfg.llm.resolve_lane().map_err(|e| anyhow::anyhow!(e))?;
    if cfg.llm.provider.eq_ignore_ascii_case("mock") && !json {
        eprintln!(
            "note: llm.provider is \"mock\"; probe still queries the HTTP lane at {}",
            resolved.base_url
        );
    }
    let provider = OpenAiCompatibleProvider::from_resolved(
        &resolved,
        cfg.llm.timeout_seconds,
        CancellationToken::new(),
    )
    .map_err(|e| anyhow::anyhow!(e))?;
    let models = provider
        .probe_models()
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "lane": resolved.name,
                "base_url": resolved.base_url,
                "configured_model": resolved.model,
                "provider": cfg.llm.provider,
                "models": models,
            })
        );
    } else {
        println!(
            "lane={}  base_url={}  configured_model={}",
            resolved.name, resolved.base_url, resolved.model
        );
        if models.is_empty() {
            println!("(no models returned)");
        } else {
            for m in models {
                println!("  {m}");
            }
        }
    }
    Ok(ExitCode::SUCCESS)
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

fn cmd_init(root: &std::path::Path, json: bool) -> Result<ExitCode> {
    let raya = root.join(".raya");
    std::fs::create_dir_all(raya.join("rules"))?;
    let example = include_str!("../../../.raya/config.toml.example");
    let config_path = raya.join("config.toml");
    if !config_path.exists() {
        std::fs::write(&config_path, example)?;
    }
    for (name, body) in [
        (
            "architecture.md",
            include_str!("../../../.raya/rules/architecture.md"),
        ),
        ("coding.md", include_str!("../../../.raya/rules/coding.md")),
        (
            "testing.md",
            include_str!("../../../.raya/rules/testing.md"),
        ),
        ("git.md", include_str!("../../../.raya/rules/git.md")),
    ] {
        let p = raya.join("rules").join(name);
        if !p.exists() {
            std::fs::write(p, body)?;
        }
    }
    let plan = raya.join("PLAN.md");
    if !plan.exists() {
        std::fs::write(
            &plan,
            "# project\n\n## Execution {#execution}\nverification: test\n\n- [ ] Describe the first task {#step-1}\n\n## decisions\n- initialized by `raya init`\n",
        )?;
    }

    let agents = root.join("AGENTS.md");
    let block = "\n\n## RAYA Agent\n\n- Prefer `raya agent run \"...\"` for coding tasks in this repo.\n- Keep durable plans in `.raya/PLAN.md` (checkbox + `{#id}` convention).\n- Sensitive tools may pause for `raya agent approve <task-id> <call-id>`.\n- Inspect history with `raya agent logs`, `raya agent activity`, and `raya agent receipt`.\n";
    if agents.exists() {
        let existing = std::fs::read_to_string(&agents)?;
        if !existing.contains("## RAYA Agent") {
            std::fs::write(&agents, format!("{existing}{block}"))?;
        }
    } else {
        std::fs::write(&agents, format!("# Agent instructions\n{block}"))?;
    }

    // Ensure DB migrates
    let cfg = Config::load(root)?;
    let _ = Store::open(cfg.database_path(root))?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "initialized": true,
                "root": root.display().to_string(),
                "config": config_path.display().to_string(),
                "plan": plan.display().to_string(),
            })
        );
    } else {
        println!("initialized RAYA project at {}", root.display());
        println!("  config: {}", config_path.display());
        println!("  plan:   {}", plan.display());
        println!("  agents: {}", agents.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn render_receipt(task: &AgentTask, events: &[(i64, raya_core::Event)]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "# RAYA task receipt");
    let _ = writeln!(out);
    let _ = writeln!(out, "- **id**: `{}`", task.id);
    let _ = writeln!(out, "- **phase**: `{:?}`", task.phase);
    let _ = writeln!(out, "- **request**: {}", task.request);
    if let Some(s) = &task.current_step {
        let _ = writeln!(out, "- **summary**: {s}");
    }
    if let Some(e) = &task.error {
        let _ = writeln!(out, "- **error**: {e}");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Timeline");
    let _ = writeln!(out);
    for (seq, e) in events {
        let _ = writeln!(
            out,
            "{}. `{}` · {} · *{}*",
            seq,
            e.kind.as_str(),
            e.evidence.as_str(),
            e.created_at.to_rfc3339()
        );
        if !e.payload.is_null() {
            let compact = serde_json::to_string(&e.payload).unwrap_or_default();
            let clipped = truncate(&compact, 200);
            let _ = writeln!(out, "   - `{clipped}`");
        }
    }
    if let Some(plan) = &task.plan {
        let _ = writeln!(out);
        let _ = writeln!(out, "## Plan");
        let _ = writeln!(out);
        for step in &plan.steps {
            let _ = writeln!(out, "- [ ] {} {{#{}}}", step.description, step.id);
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Evidence legend");
    let _ = writeln!(out, "- reported: declared by agent/runtime");
    let _ = writeln!(out, "- observed: measured (file/tool/test)");
    let _ = writeln!(out, "- inferred: ranking / heuristic");
    out
}
