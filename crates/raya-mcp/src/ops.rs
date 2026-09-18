//! Control-plane operations shared by the MCP server (and tests).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use raya_agent::Orchestrator;
use raya_core::{
    AgentTask, Config, MemoryKind, MemoryRecord, ProjectId, TaskId, TaskPhase, ToolCallId,
    redact_secrets,
};
use raya_llm::ModelRouter;
use raya_policy::PolicyEngine;
use raya_store::Store;
use raya_tools::{ToolRegistry, default_registry};
use serde_json::{Value, json};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

const LIST_LIMIT_MAX: u32 = 100;

#[derive(Debug, Error)]
pub enum McpOpError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Store(#[from] raya_store::StoreError),
    #[error(transparent)]
    Orchestrator(#[from] raya_agent::OrchestratorError),
    #[error(transparent)]
    Llm(#[from] raya_llm::LlmError),
    #[error(transparent)]
    Config(#[from] raya_core::RayaError),
}

impl McpOpError {
    pub fn msg(s: impl Into<String>) -> Self {
        Self::Message(s.into())
    }
}

/// Shared runtime wiring for MCP tools (same stack as the CLI).
#[derive(Clone)]
pub struct RayaMcpContext {
    pub store: Arc<Store>,
    pub tools: Arc<ToolRegistry>,
    pub router: Arc<ModelRouter>,
    pub config: Config,
    pub project_root: PathBuf,
    pub project_id: ProjectId,
}

impl RayaMcpContext {
    /// Open project DB and build policy/tools/router from config.
    pub fn open(project_root: &Path) -> Result<Self, McpOpError> {
        let mut config = Config::load(project_root)?;
        config
            .llm
            .apply_active_lane()
            .map_err(|e| McpOpError::msg(e.to_string()))?;
        let store = Arc::new(Store::open(config.database_path(project_root))?);
        let tools = Arc::new(default_registry(PolicyEngine::new(config.policy.clone())));
        let router = Arc::new(ModelRouter::from_config(&config, CancellationToken::new())?);
        let name = project_root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("project");
        let project = store.get_or_create_project(project_root, name)?;
        Ok(Self {
            store,
            tools,
            router,
            config,
            project_root: project_root.to_path_buf(),
            project_id: project.id,
        })
    }

    fn orch(&self) -> Orchestrator {
        Orchestrator::with_router(
            self.store.clone(),
            self.tools.clone(),
            self.router.clone(),
            self.config.clone(),
            self.project_root.clone(),
        )
    }

    pub async fn run(&self, request: &str) -> Result<Value, McpOpError> {
        let task = AgentTask::new(
            self.project_id,
            request,
            self.config.agent.max_iterations,
            self.config.agent.max_tool_calls,
            self.config.context.max_tokens.saturating_mul(4),
            self.config.context.max_tokens,
            Some(Utc::now() + chrono::Duration::seconds(self.config.agent.timeout_seconds as i64)),
        );
        let id = task.id;
        self.store.create_task(&task)?;
        let finished = self.orch().run(id, CancellationToken::new()).await?;
        Ok(task_json(&finished))
    }

    pub fn status(&self, task_id: Option<&str>) -> Result<Value, McpOpError> {
        if let Some(id) = task_id {
            let tid: TaskId = id
                .parse()
                .map_err(|e| McpOpError::msg(format!("task id: {e}")))?;
            let task = self
                .store
                .get_task(tid)?
                .ok_or_else(|| McpOpError::msg("task not found"))?;
            Ok(task_json(&task))
        } else {
            let tasks = self.store.list_tasks(Some(self.project_id), 20)?;
            Ok(json!({
                "tasks": tasks.iter().map(task_json).collect::<Vec<_>>(),
            }))
        }
    }

    pub fn logs(&self, task_id: &str, limit: Option<u32>) -> Result<Value, McpOpError> {
        let tid: TaskId = task_id
            .parse()
            .map_err(|e| McpOpError::msg(format!("task id: {e}")))?;
        let limit = limit.unwrap_or(50).clamp(1, LIST_LIMIT_MAX);
        let events = self.store.list_events(tid, None, limit)?;
        let rows: Vec<Value> = events
            .into_iter()
            .map(|(seq, e)| {
                json!({
                    "seq": seq,
                    "kind": e.kind.as_str(),
                    "evidence": e.evidence.as_str(),
                    "payload": redact_event_payload(&e.payload),
                    "created_at": e.created_at.to_rfc3339(),
                })
            })
            .collect();
        Ok(json!({ "events": rows }))
    }

    pub async fn approve(
        &self,
        task_id: &str,
        call_id: &str,
        granted: bool,
        no_resume: bool,
    ) -> Result<Value, McpOpError> {
        let tid: TaskId = task_id
            .parse()
            .map_err(|e| McpOpError::msg(format!("task id: {e}")))?;
        let cid: ToolCallId = call_id
            .parse()
            .map_err(|e| McpOpError::msg(format!("call id: {e}")))?;
        let task = self
            .store
            .get_task(tid)?
            .ok_or_else(|| McpOpError::msg("task not found"))?;
        if task.phase != TaskPhase::WaitingApproval {
            return Err(McpOpError::msg(format!(
                "task is not waiting for approval (phase={:?})",
                task.phase
            )));
        }
        let cp = self
            .store
            .load_checkpoint(tid)?
            .ok_or_else(|| McpOpError::msg("no checkpoint for task; cannot approve"))?;
        let pending_id = cp.pending_call.as_ref().map(|c| c.id);
        if pending_id != Some(cid) {
            return Err(McpOpError::msg(format!(
                "call_id does not match pending approval (expected {pending_id:?})"
            )));
        }
        self.store.set_approval(tid, cid, granted)?;
        if no_resume {
            return Ok(json!({
                "approved": granted,
                "task_id": task_id,
                "call_id": call_id,
                "resumed": false,
            }));
        }
        let finished = self.orch().run(tid, CancellationToken::new()).await?;
        Ok(json!({
            "approved": granted,
            "task_id": task_id,
            "call_id": call_id,
            "resumed": true,
            "task": task_json(&finished),
        }))
    }

    pub async fn resume(&self, task_id: &str) -> Result<Value, McpOpError> {
        let tid: TaskId = task_id
            .parse()
            .map_err(|e| McpOpError::msg(format!("task id: {e}")))?;
        let finished = self.orch().run(tid, CancellationToken::new()).await?;
        Ok(task_json(&finished))
    }

    pub fn cancel(&self, task_id: &str) -> Result<Value, McpOpError> {
        let tid: TaskId = task_id
            .parse()
            .map_err(|e| McpOpError::msg(format!("task id: {e}")))?;
        let ok = self.store.request_cancel(tid)?;
        Ok(json!({ "cancelled": ok, "task_id": task_id }))
    }

    pub fn dag(&self, task_id: &str) -> Result<Value, McpOpError> {
        let tid: TaskId = task_id
            .parse()
            .map_err(|e| McpOpError::msg(format!("task id: {e}")))?;
        let _ = self
            .store
            .get_task(tid)?
            .ok_or_else(|| McpOpError::msg("task not found"))?;
        let nodes = self.store.list_dag_nodes(tid)?;
        let rows: Vec<Value> = nodes
            .into_iter()
            .map(|n| {
                json!({
                    "node_id": n.node_id,
                    "role": n.role,
                    "status": n.status.as_str(),
                    "summary": n.summary.as_deref().map(redact_secrets),
                    "depends_on": n.depends_on,
                })
            })
            .collect();
        Ok(json!({ "nodes": rows }))
    }

    pub fn memory_list(&self, kind: Option<&str>, limit: Option<u32>) -> Result<Value, McpOpError> {
        let kind = match kind {
            Some(k) => Some(
                MemoryKind::parse(k)
                    .ok_or_else(|| McpOpError::msg(format!("unknown kind: {k}")))?,
            ),
            None => None,
        };
        let limit = limit.unwrap_or(20).clamp(1, LIST_LIMIT_MAX);
        let rows = self.store.list_memories(self.project_id, kind, limit)?;
        Ok(json!({ "memories": rows.iter().map(memory_json).collect::<Vec<_>>() }))
    }

    pub fn memory_search(&self, query: &str, limit: Option<u32>) -> Result<Value, McpOpError> {
        let limit = limit.unwrap_or(20).clamp(1, LIST_LIMIT_MAX);
        let rows = self.store.search_memories(self.project_id, query, limit)?;
        Ok(json!({ "memories": rows.iter().map(memory_json).collect::<Vec<_>>() }))
    }

    pub fn memory_add(&self, text: &str, kind: Option<&str>) -> Result<Value, McpOpError> {
        let kind = MemoryKind::parse(kind.unwrap_or("project"))
            .ok_or_else(|| McpOpError::msg("unknown memory kind"))?;
        let mem = MemoryRecord::new(self.project_id, kind, redact_secrets(text), None);
        let id = mem.id.clone();
        self.store.upsert_memory(&mem)?;
        Ok(json!({ "id": id, "kind": kind.as_str() }))
    }

    pub fn memory_forget(&self, id: &str) -> Result<Value, McpOpError> {
        let ok = self.store.delete_memory(id)?;
        Ok(json!({ "deleted": ok, "id": id }))
    }
}

fn task_json(task: &AgentTask) -> Value {
    json!({
        "id": task.id.to_string(),
        "phase": serde_json::to_value(task.phase).unwrap_or(Value::Null),
        "status": serde_json::to_value(task.status()).unwrap_or(Value::Null),
        "request": redact_secrets(&task.request),
        "summary": task.current_step.as_deref().map(redact_secrets),
        "error": task.error.as_deref().map(redact_secrets),
        "iterations": task.iterations,
        "tool_calls": task.tool_calls,
    })
}

fn memory_json(m: &MemoryRecord) -> Value {
    json!({
        "id": m.id,
        "kind": m.kind.as_str(),
        "key": m.key,
        "content": redact_secrets(&m.content),
        "updated_at": m.updated_at.to_rfc3339(),
    })
}

fn redact_event_payload(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(redact_secrets(s)),
        Value::Array(items) => Value::Array(items.iter().map(redact_event_payload).collect()),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, val) in map {
                out.insert(k.clone(), redact_event_payload(val));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Stable tool names exposed over MCP.
pub fn tool_names() -> &'static [&'static str] {
    &[
        "raya_run",
        "raya_status",
        "raya_logs",
        "raya_approve",
        "raya_resume",
        "raya_cancel",
        "raya_dag",
        "raya_memory_list",
        "raya_memory_search",
        "raya_memory_add",
        "raya_memory_forget",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_include_run() {
        assert!(tool_names().contains(&"raya_run"));
        assert!(tool_names().contains(&"raya_approve"));
    }
}
