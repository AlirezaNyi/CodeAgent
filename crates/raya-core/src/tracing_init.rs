//! Tracing / logging initialization.

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;

/// Log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Pretty,
    Json,
}

impl LogFormat {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "json" => Self::Json,
            _ => Self::Pretty,
        }
    }
}

/// Initialize the global tracing subscriber (stdout).
///
/// Safe to call once at process start. Subsequent calls are ignored.
pub fn init_tracing(format: LogFormat) {
    init_tracing_inner(format, false);
}

/// Initialize tracing to **stderr** (required for stdio MCP so JSON-RPC stays clean on stdout).
pub fn init_tracing_stderr(format: LogFormat) {
    init_tracing_inner(format, true);
}

fn init_tracing_inner(format: LogFormat, stderr: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let result = match (format, stderr) {
        (LogFormat::Json, true) => fmt()
            .json()
            .with_writer(std::io::stderr)
            .with_env_filter(filter)
            .with_target(true)
            .try_init(),
        (LogFormat::Json, false) => fmt()
            .json()
            .with_env_filter(filter)
            .with_target(true)
            .try_init(),
        (LogFormat::Pretty, true) => fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(filter)
            .with_target(false)
            .try_init(),
        (LogFormat::Pretty, false) => fmt().with_env_filter(filter).with_target(false).try_init(),
    };

    // Ignore "already initialized" when tests or nested entrypoints call this.
    let _ = result;
}
