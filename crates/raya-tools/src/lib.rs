//! Tool runtime: trait, registry, path safety, and built-in tools.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod path;
mod registry;
mod tools;

pub use path::{PathError, safe_path};
pub use registry::{ToolContext, ToolError, ToolRegistry, default_registry};
pub use tools::*;

use async_trait::async_trait;
use raya_core::{ToolResult, ToolSchema};
use serde_json::Value;

/// Common tool interface.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> ToolSchema;

    async fn execute(&self, ctx: &ToolContext, input: Value) -> Result<ToolResult, ToolError>;
}
