//! MCP stdio server wrapping [`crate::ops::RayaMcpContext`].

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig};
use rmcp::transport::stdio;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::Value;

use crate::ops::RayaMcpContext;

fn op_err(e: impl std::fmt::Display) -> McpError {
    McpError::invalid_params(e.to_string(), None)
}

fn text_result(v: Value) -> Result<CallToolResult, McpError> {
    let s = serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string());
    Ok(CallToolResult::success(vec![ContentBlock::text(s)]))
}

#[derive(Clone)]
pub struct RayaMcpServer {
    ctx: Arc<RayaMcpContext>,
    #[allow(dead_code)] // read by #[tool_handler] generated code
    tool_router: rmcp::handler::server::tool::ToolRouter<Self>,
}

impl RayaMcpServer {
    pub fn new(ctx: Arc<RayaMcpContext>) -> Self {
        Self {
            ctx,
            tool_router: Self::tool_router(),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RunArgs {
    /// Natural-language coding task.
    pub request: String,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct StatusArgs {
    pub task_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LogsArgs {
    pub task_id: String,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ApproveArgs {
    pub task_id: String,
    pub call_id: String,
    /// Defaults to true (grant).
    pub granted: Option<bool>,
    /// If true, only record approval without resuming.
    pub no_resume: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TaskIdArgs {
    pub task_id: String,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct MemoryListArgs {
    pub kind: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemorySearchArgs {
    pub query: String,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryAddArgs {
    pub text: String,
    /// Defaults to `project`.
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryForgetArgs {
    pub id: String,
}

#[tool_router]
impl RayaMcpServer {
    #[tool(
        name = "raya_run",
        description = "Create and run a RAYA coding task (mock/OpenAI LLM + policy-gated tools)"
    )]
    async fn raya_run(
        &self,
        Parameters(args): Parameters<RunArgs>,
    ) -> Result<CallToolResult, McpError> {
        let v = self.ctx.run(&args.request).await.map_err(op_err)?;
        text_result(v)
    }

    #[tool(
        name = "raya_status",
        description = "Show one task status, or list recent tasks when task_id is omitted"
    )]
    async fn raya_status(
        &self,
        Parameters(args): Parameters<StatusArgs>,
    ) -> Result<CallToolResult, McpError> {
        let v = self.ctx.status(args.task_id.as_deref()).map_err(op_err)?;
        text_result(v)
    }

    #[tool(name = "raya_logs", description = "List structured events for a task")]
    async fn raya_logs(
        &self,
        Parameters(args): Parameters<LogsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let v = self.ctx.logs(&args.task_id, args.limit).map_err(op_err)?;
        text_result(v)
    }

    #[tool(
        name = "raya_approve",
        description = "Approve or deny a pending tool call; resumes the task by default"
    )]
    async fn raya_approve(
        &self,
        Parameters(args): Parameters<ApproveArgs>,
    ) -> Result<CallToolResult, McpError> {
        let v = self
            .ctx
            .approve(
                &args.task_id,
                &args.call_id,
                args.granted.unwrap_or(true),
                args.no_resume.unwrap_or(false),
            )
            .await
            .map_err(op_err)?;
        text_result(v)
    }

    #[tool(
        name = "raya_resume",
        description = "Resume a task waiting for approval from its checkpoint"
    )]
    async fn raya_resume(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        let v = self.ctx.resume(&args.task_id).await.map_err(op_err)?;
        text_result(v)
    }

    #[tool(
        name = "raya_cancel",
        description = "Request cancellation of a running task"
    )]
    async fn raya_cancel(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        let v = self.ctx.cancel(&args.task_id).map_err(op_err)?;
        text_result(v)
    }

    #[tool(
        name = "raya_dag",
        description = "Show optional execution DAG node status for a task"
    )]
    async fn raya_dag(
        &self,
        Parameters(args): Parameters<TaskIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        let v = self.ctx.dag(&args.task_id).map_err(op_err)?;
        text_result(v)
    }

    #[tool(
        name = "raya_memory_list",
        description = "List recent project memories (bounded)"
    )]
    async fn raya_memory_list(
        &self,
        Parameters(args): Parameters<MemoryListArgs>,
    ) -> Result<CallToolResult, McpError> {
        let v = self
            .ctx
            .memory_list(args.kind.as_deref(), args.limit)
            .map_err(op_err)?;
        text_result(v)
    }

    #[tool(
        name = "raya_memory_search",
        description = "Search project memories with FTS (bounded)"
    )]
    async fn raya_memory_search(
        &self,
        Parameters(args): Parameters<MemorySearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let v = self
            .ctx
            .memory_search(&args.query, args.limit)
            .map_err(op_err)?;
        text_result(v)
    }

    #[tool(
        name = "raya_memory_add",
        description = "Add a curated project memory note"
    )]
    async fn raya_memory_add(
        &self,
        Parameters(args): Parameters<MemoryAddArgs>,
    ) -> Result<CallToolResult, McpError> {
        let v = self
            .ctx
            .memory_add(&args.text, args.kind.as_deref())
            .map_err(op_err)?;
        text_result(v)
    }

    #[tool(name = "raya_memory_forget", description = "Delete a memory by id")]
    async fn raya_memory_forget(
        &self,
        Parameters(args): Parameters<MemoryForgetArgs>,
    ) -> Result<CallToolResult, McpError> {
        let v = self.ctx.memory_forget(&args.id).map_err(op_err)?;
        text_result(v)
    }
}

#[tool_handler]
impl ServerHandler for RayaMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "RAYA Agent control plane: run coding tasks, approve tools, inspect memory/DAG. \
                 File/shell tools stay inside the agent and are policy-gated.",
        )
    }
}

/// Serve MCP over stdin/stdout until the client disconnects.
pub async fn serve_stdio(ctx: Arc<RayaMcpContext>) -> anyhow::Result<()> {
    let server = RayaMcpServer::new(ctx);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
