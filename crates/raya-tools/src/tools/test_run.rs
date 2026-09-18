//! test.run tool with project-type detection.

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use raya_core::{ToolCallId, ToolResult, ToolSchema};
use raya_executor::{ProcessSpec, run_process};
use serde_json::{Value, json};

use crate::Tool;
use crate::registry::{ToolContext, ToolError};

pub struct TestRunTool;

#[async_trait]
impl Tool for TestRunTool {
    fn name(&self) -> &str {
        "test.run"
    }

    fn description(&self) -> &str {
        "Run project tests (Cargo / npm / pytest)"
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

    async fn execute(&self, ctx: &ToolContext, input: Value) -> Result<ToolResult, ToolError> {
        let command = if let Some(c) = input.get("command").and_then(|v| v.as_str()) {
            c.to_string()
        } else {
            detect_test_command(&ctx.project_root)
                .ok_or_else(|| ToolError::Message("could not detect test command".into()))?
        };
        run_command(ctx, "test.run", &command).await
    }
}

pub(crate) fn detect_test_command(root: &Path) -> Option<String> {
    if root.join("Cargo.toml").exists() {
        return Some("cargo test".into());
    }
    if root.join("package.json").exists() {
        return Some("npm test".into());
    }
    if root.join("pytest.ini").exists()
        || root.join("pyproject.toml").exists()
        || root.join("tests").is_dir()
    {
        return Some("python -m pytest".into());
    }
    None
}

pub(crate) async fn run_command(
    ctx: &ToolContext,
    name: &str,
    command: &str,
) -> Result<ToolResult, ToolError> {
    let spec = ProcessSpec::new("sh")
        .args(["-c", command])
        .cwd(ctx.project_root.clone())
        .timeout(Duration::from_secs(ctx.process_timeout_secs))
        .max_stdout(ctx.max_output_bytes)
        .max_stderr(ctx.max_output_bytes);

    let out = run_process(spec, ctx.cancel.clone())
        .await
        .map_err(|e| ToolError::Executor(e.to_string()))?;

    if out.cancelled {
        return Err(ToolError::Cancelled);
    }

    let mut text = format!("$ {command}\n");
    text.push_str(&out.stdout);
    if !out.stderr.is_empty() {
        text.push_str("\nstderr:\n");
        text.push_str(&out.stderr);
    }

    Ok(ToolResult {
        call_id: ToolCallId::new(),
        name: name.into(),
        success: out.exit_code == Some(0) && !out.timed_out,
        output: text,
        truncated: out.truncated,
        error: if out.exit_code == Some(0) {
            None
        } else {
            Some(format!("exit {:?}", out.exit_code))
        },
        duration_ms: out.duration.as_millis() as u64,
    })
}
