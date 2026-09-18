//! LLM provider abstraction and implementations.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod mock;
mod openai;
mod provider;
mod router;

pub use mock::MockProvider;
pub use openai::{OpenAiCompatibleProvider, is_loopback_base_url};
pub use provider::{LlmError, LlmProvider, LlmResult};
pub use router::{ModelRole, ModelRouter};
