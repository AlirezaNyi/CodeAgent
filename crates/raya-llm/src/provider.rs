//! Provider trait and errors.

use async_trait::async_trait;
use raya_core::{CompletionRequest, CompletionResponse};
use thiserror::Error;

pub type LlmResult<T> = Result<T, LlmError>;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("LLM request timed out")]
    Timeout,

    #[error("LLM rate limited")]
    RateLimited,

    #[error("LLM authentication failed")]
    Auth,

    #[error("invalid LLM response: {0}")]
    InvalidResponse(String),

    #[error("LLM request cancelled")]
    Cancelled,

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("{0}")]
    Message(String),
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> LlmResult<CompletionResponse>;
}
