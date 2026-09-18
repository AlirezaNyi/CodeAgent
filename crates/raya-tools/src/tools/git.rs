//! Git status / diff tools via bounded executor.

use std::time::Duration;

use async_trait::async_trait;
use raya_core::{ToolCallId, ToolResult, ToolSchema};
use raya_executor::{ProcessSpec, run_process};
use serde_json::{Value, json};

use crate::Tool;
use crate::registry::{ToolContext, ToolError};

pub struct GitStatusTool;

#[async_trait]
impl Tool for GitStatusTool {
    fn name(&self) -> &str {
        "git.status"
    }

    fn description(&self) -> &str {
        "Show git status --short"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({"type": "object", "properties": {}}),
        }
    }

    async fn execute(&self, ctx: &ToolContext, _input: Value) -> Result<ToolResult, ToolError> {
        run_git(ctx, &["status", "--short"]).await
    }
}

pub struct GitDiffTool;

#[async_trait]
impl Tool for GitDiffTool {
    fn name(&self) -> &str {
        "git.diff"
    }

    fn description(&self) -> &str {
        "Show git diff (working tree)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "staged": {"type": "boolean"}
                }
            }),
        }
    }

    async fn execute(&self, ctx: &ToolContext, input: Value) -> Result<ToolResult, ToolError> {
        let staged = input
            .get("staged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if staged {
            run_git(ctx, &["diff", "--cached"]).await
        } else {
            run_git(ctx, &["diff"]).await
        }
    }
}

async fn run_git(ctx: &ToolContext, args: &[&str]) -> Result<ToolResult, ToolError> {
    let mut spec = ProcessSpec::new("git")
        .args(args.iter().map(|s| (*s).to_string()))
        .cwd(ctx.project_root.clone())
        .timeout(Duration::from_secs(ctx.process_timeout_secs.min(60)))
        .max_stdout(ctx.max_output_bytes)
        .max_stderr(ctx.max_output_bytes);
    spec.env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());

    let out = run_process(spec, ctx.cancel.clone())
        .await
        .map_err(|e| ToolError::Executor(e.to_string()))?;

    if out.cancelled {
        return Err(ToolError::Cancelled);
    }

    let mut text = out.stdout;
    if !out.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&out.stderr);
    }

    Ok(ToolResult {
        call_id: ToolCallId::new(),
        name: "git.status".into(),
        success: out.exit_code == Some(0),
        output: text,
        truncated: out.truncated,
        error: if out.exit_code == Some(0) {
            None
        } else {
            Some(format!("git exited {:?}", out.exit_code))
        },
        duration_ms: out.duration.as_millis() as u64,
    })
}
