//! Bounded shell.exec tool.

use std::time::Duration;

use async_trait::async_trait;
use raya_core::{ToolCallId, ToolResult, ToolSchema};
use raya_executor::{ProcessSpec, run_process};
use serde_json::{Value, json};

use crate::Tool;
use crate::registry::{ToolContext, ToolError};

pub struct ShellExecTool;

#[async_trait]
impl Tool for ShellExecTool {
    fn name(&self) -> &str {
        "shell.exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command in the project root (policy-controlled)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "timeout_seconds": {"type": "integer"}
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(&self, ctx: &ToolContext, input: Value) -> Result<ToolResult, ToolError> {
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("command required".into()))?;
        let timeout = input
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(ctx.process_timeout_secs);

        let spec = ProcessSpec::new("sh")
            .args(["-c", command])
            .cwd(ctx.project_root.clone())
            .timeout(Duration::from_secs(timeout))
            .max_stdout(ctx.max_output_bytes)
            .max_stderr(ctx.max_output_bytes);

        let out = run_process(spec, ctx.cancel.clone())
            .await
            .map_err(|e| ToolError::Executor(e.to_string()))?;

        if out.cancelled {
            return Err(ToolError::Cancelled);
        }

        let mut text = String::new();
        if !out.stdout.is_empty() {
            text.push_str(&out.stdout);
        }
        if !out.stderr.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str("stderr:\n");
            text.push_str(&out.stderr);
        }
        if out.timed_out {
            text.push_str("\n[timed out]");
        }

        Ok(ToolResult {
            call_id: ToolCallId::new(),
            name: self.name().into(),
            success: out.exit_code == Some(0) && !out.timed_out,
            output: text,
            truncated: out.truncated,
            error: if out.exit_code == Some(0) && !out.timed_out {
                None
            } else {
                Some(format!(
                    "exit={:?} timed_out={}",
                    out.exit_code, out.timed_out
                ))
            },
            duration_ms: out.duration.as_millis() as u64,
        })
    }
}
