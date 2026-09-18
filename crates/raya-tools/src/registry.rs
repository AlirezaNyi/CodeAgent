//! Tool registry and execution context.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use raya_core::{TaskId, ToolCall, ToolCallId, ToolResult, ToolSchema};
use raya_policy::PolicyEngine;
use serde_json::Value;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::Tool;
use crate::path::PathError;
use crate::tools::{
    BuildRunTool, FilesystemPatchTool, FilesystemReadTool, FilesystemWriteTool, GitDiffTool,
    GitStatusTool, SearchGrepTool, ShellExecTool, TestRunTool,
};

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("unknown tool: {0}")]
    Unknown(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error(transparent)]
    Path(#[from] PathError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("executor error: {0}")]
    Executor(String),

    #[error("policy denied: {0}")]
    Denied(String),

    #[error("approval required: {0}")]
    ApprovalRequired(String),

    #[error("cancelled")]
    Cancelled,

    #[error("{0}")]
    Message(String),
}

/// Context passed to every tool invocation.
#[derive(Clone)]
pub struct ToolContext {
    pub task_id: TaskId,
    pub project_root: PathBuf,
    pub cancel: CancellationToken,
    pub max_output_bytes: usize,
    pub process_timeout_secs: u64,
}

impl ToolContext {
    pub fn new(task_id: TaskId, project_root: PathBuf, cancel: CancellationToken) -> Self {
        Self {
            task_id,
            project_root,
            cancel,
            max_output_bytes: 256 * 1024,
            process_timeout_secs: 300,
        }
    }
}

/// Name → tool registry.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    policy: PolicyEngine,
}

impl ToolRegistry {
    pub fn new(policy: PolicyEngine) -> Self {
        Self {
            tools: HashMap::new(),
            policy,
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| t.schema()).collect()
    }

    pub fn policy(&self) -> &PolicyEngine {
        &self.policy
    }

    /// Evaluate policy then execute.
    pub async fn execute(
        &self,
        ctx: &ToolContext,
        call: &ToolCall,
    ) -> Result<ToolResult, ToolError> {
        self.execute_inner(ctx, call, false).await
    }

    /// Like [`Self::execute`], but treat prior human approval as allow
    /// (still honors Deny).
    pub async fn execute_approved(
        &self,
        ctx: &ToolContext,
        call: &ToolCall,
    ) -> Result<ToolResult, ToolError> {
        self.execute_inner(ctx, call, true).await
    }

    async fn execute_inner(
        &self,
        ctx: &ToolContext,
        call: &ToolCall,
        already_approved: bool,
    ) -> Result<ToolResult, ToolError> {
        if ctx.cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }

        let decision = self.policy.evaluate(&call.name, &call.input);
        match decision {
            raya_core::PolicyDecision::Allow => {}
            raya_core::PolicyDecision::Deny { reason } => {
                warn!(tool = %call.name, %reason, "tool denied");
                return Err(ToolError::Denied(reason));
            }
            raya_core::PolicyDecision::RequireApproval { reason } => {
                if already_approved {
                    info!(tool = %call.name, "tool previously approved; executing");
                } else {
                    return Err(ToolError::ApprovalRequired(reason));
                }
            }
        }

        let tool = self
            .get(&call.name)
            .ok_or_else(|| ToolError::Unknown(call.name.clone()))?;

        info!(tool = %call.name, call_id = %call.id, "tool starting");
        let start = Instant::now();
        let mut result = tool.execute(ctx, call.input.clone()).await?;
        result.call_id = call.id;
        result.duration_ms = start.elapsed().as_millis() as u64;

        // Bound output size
        if result.output.len() > ctx.max_output_bytes {
            result.output.truncate(ctx.max_output_bytes);
            result.truncated = true;
        }

        Ok(result)
    }

    /// Execute by name without a prebuilt ToolCall.
    pub async fn execute_named(
        &self,
        ctx: &ToolContext,
        name: &str,
        input: Value,
    ) -> Result<ToolResult, ToolError> {
        let call = ToolCall {
            id: ToolCallId::new(),
            name: name.to_string(),
            input,
        };
        self.execute(ctx, &call).await
    }
}

/// Default set of MVP tools.
pub fn default_registry(policy: PolicyEngine) -> ToolRegistry {
    let mut reg = ToolRegistry::new(policy);
    reg.register(Arc::new(FilesystemReadTool));
    reg.register(Arc::new(FilesystemWriteTool));
    reg.register(Arc::new(FilesystemPatchTool));
    reg.register(Arc::new(SearchGrepTool));
    reg.register(Arc::new(GitStatusTool));
    reg.register(Arc::new(GitDiffTool));
    reg.register(Arc::new(ShellExecTool));
    reg.register(Arc::new(TestRunTool));
    reg.register(Arc::new(BuildRunTool));
    reg
}
