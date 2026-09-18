//! build.run tool.

use async_trait::async_trait;
use raya_core::ToolSchema;
use serde_json::{Value, json};

use crate::Tool;
use crate::registry::{ToolContext, ToolError};
use crate::tools::test_run::{detect_test_command, run_command};

pub struct BuildRunTool;

#[async_trait]
impl Tool for BuildRunTool {
    fn name(&self) -> &str {
        "build.run"
    }

    fn description(&self) -> &str {
        "Build the project (Cargo / npm)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                }
            }),
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        input: Value,
    ) -> Result<raya_core::ToolResult, ToolError> {
        let command = if let Some(c) = input.get("command").and_then(|v| v.as_str()) {
            c.to_string()
        } else {
            detect_build_command(&ctx.project_root)
                .ok_or_else(|| ToolError::Message("could not detect build command".into()))?
        };
        run_command(ctx, "build.run", &command).await
    }
}

fn detect_build_command(root: &std::path::Path) -> Option<String> {
    if root.join("Cargo.toml").exists() {
        return Some("cargo build".into());
    }
    if root.join("package.json").exists() {
        return Some("npm run build".into());
    }
    // Fall back: if tests are detectable, do nothing special
    let _ = detect_test_command(root);
    None
}
