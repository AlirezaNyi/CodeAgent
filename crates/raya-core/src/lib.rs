//! Core types, configuration, errors, and utilities for RAYA Agent.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod config;
pub mod error;
pub mod models;
pub mod planio;
pub mod project;
pub mod redact;
pub mod tokens;
pub mod tracing_init;

pub use config::{
    AgentConfig, Config, ContextConfig, GitConfig, LlmConfig, LlmLane, PolicyAction, PolicyConfig,
    ResolvedLlmLane, ResourcesConfig, ServerConfig, SubagentConfig,
};
pub use error::{ConfigError, RayaError, Result};
pub use models::*;
pub use planio::{
    PLAN_REL_PATH, plan_from_markdown, plan_to_markdown, read_plan_file, write_plan_file,
};
pub use project::{ProjectRoot, discover};
pub use redact::redact_secrets;
pub use tokens::{HeuristicCounter, TokenCounter};
pub use tracing_init::{LogFormat, init_tracing};
