//! Typed error hierarchy for RAYA Agent.

use thiserror::Error;

/// Convenient result alias using [`RayaError`].
pub type Result<T> = std::result::Result<T, RayaError>;

/// Top-level error type for library crates.
#[derive(Debug, Error)]
pub enum RayaError {
    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Project(#[from] ProjectError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Message(String),
}

/// Configuration loading and validation errors.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config at {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid TOML in config at {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },

    #[error("invalid configuration: {0}")]
    Validation(String),
}

/// Project discovery errors.
#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("could not determine current working directory: {0}")]
    Cwd(#[source] std::io::Error),

    #[error("no project root found from {start} (looked for .git or .raya)")]
    NotFound { start: String },
}
