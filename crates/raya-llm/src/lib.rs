//! LLM provider abstraction and implementations.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod mock;
mod openai;
mod provider;

pub use mock::MockProvider;
pub use openai::OpenAiCompatibleProvider;
pub use provider::{LlmError, LlmProvider, LlmResult};
