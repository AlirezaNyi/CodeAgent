//! Store error types.

use thiserror::Error;

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("migration error: {0}")]
    Migration(#[from] rusqlite_migration::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("invalid uuid: {0}")]
    Uuid(#[from] uuid::Error),

    #[error("invalid task phase: {0}")]
    InvalidPhase(String),

    #[error("invalid event kind: {0}")]
    InvalidEventKind(String),

    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("project not found: {0}")]
    ProjectNotFound(String),

    #[error("invalid phase transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: raya_core::TaskPhase,
        to: raya_core::TaskPhase,
    },

    #[error("{0}")]
    Message(String),
}
