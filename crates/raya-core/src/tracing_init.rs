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

/// Initialize the global tracing subscriber.
///
/// Safe to call once at process start. Subsequent calls are ignored.
pub fn init_tracing(format: LogFormat) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let result = match format {
        LogFormat::Json => fmt()
            .json()
            .with_env_filter(filter)
            .with_target(true)
            .try_init(),
        LogFormat::Pretty => fmt().with_env_filter(filter).with_target(false).try_init(),
    };

    // Ignore "already initialized" when tests or nested entrypoints call this.
    let _ = result;
}
