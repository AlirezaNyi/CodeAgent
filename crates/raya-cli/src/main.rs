//! RAYA Agent CLI.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use raya_core::{Config, LogFormat, discover, init_tracing};
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
        /// Bind host (default from config).
        #[arg(long)]
        host: Option<String>,
        /// Bind port (default from config).
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
    Status {
        /// Task ID (UUID). Omit to list recent tasks.
        task_id: Option<String>,
    },
    /// Show structured task events.
    Logs {
        task_id: String,
        /// Follow new events (not yet implemented in bootstrap).
        #[arg(long)]
        follow: bool,
    },
    /// Cancel a running task.
    Cancel { task_id: String },
    /// Approve a pending tool call.
    Approve { task_id: String, call_id: String },
    /// Index the repository (MVP stub).
    Index,
    /// Explain a file with bounded context.
    Explain { file: PathBuf },
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    let project = discover(cli.project.as_deref()).context("project discovery failed")?;
    let config = Config::load(project.path()).context("failed to load config")?;

    let format = LogFormat::parse(&config.server.log_format);
    init_tracing(format);

    info!(
        project = %project.path().display(),
        "raya starting"
    );

    match cli.command {
        Commands::Agent { command } => match command {
            AgentCommands::Run { task } => {
                if cli.json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "status": "not_implemented",
                            "command": "agent.run",
                            "task": task,
                            "project": project.path().display().to_string(),
                        })
                    );
                } else {
                    println!("agent run is not yet implemented (task queued for Phase 1 T9/T10)");
                    println!("project: {}", project.path().display());
                    println!("task: {task}");
                }
            }
            AgentCommands::Status { task_id } => {
                println!(
                    "agent status is not yet implemented (task_id={})",
                    task_id.as_deref().unwrap_or("(latest)")
                );
            }
            AgentCommands::Logs { task_id, follow } => {
                println!("agent logs is not yet implemented (task_id={task_id}, follow={follow})");
            }
            AgentCommands::Cancel { task_id } => {
                println!("agent cancel is not yet implemented (task_id={task_id})");
            }
            AgentCommands::Approve { task_id, call_id } => {
                println!(
                    "agent approve is not yet implemented (task_id={task_id}, call_id={call_id})"
                );
            }
            AgentCommands::Index => {
                println!("agent index is not yet implemented");
            }
            AgentCommands::Explain { file } => {
                println!(
                    "agent explain is not yet implemented (file={})",
                    file.display()
                );
            }
        },
        Commands::Serve { host, port } => {
            let host = host.unwrap_or_else(|| config.server.host.clone());
            let port = port.unwrap_or(config.server.port);
            println!("HTTP server is not yet implemented (would bind {host}:{port})");
        }
    }

    Ok(())
}
