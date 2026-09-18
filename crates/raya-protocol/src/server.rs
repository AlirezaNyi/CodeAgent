//! Axum HTTP server.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use raya_agent::Orchestrator;
use raya_core::{AgentTask, Config, MemoryKind, ProjectId, TaskId, TaskPhase, ToolCallId};
use raya_llm::ModelRouter;
use raya_store::Store;
use raya_tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub tools: Arc<ToolRegistry>,
    pub router: Arc<ModelRouter>,
    pub config: Config,
    pub project_root: PathBuf,
    pub started: Instant,
    pub task_slots: Arc<Semaphore>,
    pub cancel_tokens:
        Arc<tokio::sync::Mutex<std::collections::HashMap<TaskId, CancellationToken>>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    pub request: String,
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateTaskResponse {
    pub id: String,
    pub phase: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub root_path: String,
    pub name: String,
}

pub fn bind_addr(config: &Config) -> Result<SocketAddr, String> {
    let host: IpAddr = config
        .server
        .host
        .parse()
        .map_err(|e| format!("invalid host: {e}"))?;
    if !config.server.bind_loopback && !config.server.allow_remote {
        return Err("allow_remote required for non-loopback bind".into());
    }
    if config.server.bind_loopback
        && !host.is_loopback()
        && host != IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    {
        // still allow explicit config host if loopback flag set only for default
    }
    Ok(SocketAddr::new(host, config.server.port))
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/v1/tasks", post(create_task))
        .route("/v1/tasks/{id}", get(get_task))
        .route("/v1/tasks/{id}/cancel", post(cancel_task))
        .route("/v1/tasks/{id}/approve", post(approve_task))
        .route("/v1/tasks/{id}/events", get(list_events))
        .route("/v1/projects", post(create_project))
        .route("/v1/projects/{id}/index", post(index_project))
        .route("/v1/projects/{id}/memory", get(list_project_memory))
        .with_state(state)
}

pub async fn serve(state: AppState) -> Result<(), String> {
    let addr = bind_addr(&state.config)?;
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| e.to_string())?;
    info!(%addr, "HTTP API listening");
    axum::serve(listener, app).await.map_err(|e| e.to_string())
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok"}))
}

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let tasks = state.store.list_tasks(None, 1000).unwrap_or_default();
    let mut completed = 0;
    let mut failed = 0;
    let mut running = 0;
    let mut cancelled = 0;
    for t in &tasks {
        match t.phase {
            TaskPhase::Completed => completed += 1,
            TaskPhase::Failed => failed += 1,
            TaskPhase::Cancelled => cancelled += 1,
            _ => running += 1,
        }
    }
    let body = format!(
        "# TYPE raya_uptime_seconds gauge\nraya_uptime_seconds {}\n\
         # TYPE raya_tasks_completed gauge\nraya_tasks_completed {completed}\n\
         # TYPE raya_tasks_failed gauge\nraya_tasks_failed {failed}\n\
         # TYPE raya_tasks_running gauge\nraya_tasks_running {running}\n\
         # TYPE raya_tasks_cancelled gauge\nraya_tasks_cancelled {cancelled}\n\
         # TYPE raya_task_slots_available gauge\nraya_task_slots_available {}\n",
        state.started.elapsed().as_secs(),
        state.task_slots.available_permits()
    );
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        body,
    )
}

async fn create_task(
    State(state): State<AppState>,
    Json(body): Json<CreateTaskRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if body.request.trim().is_empty() {
        return Err(ApiError::bad("request must not be empty"));
    }
    let project = if let Some(pid) = body.project_id {
        let id: ProjectId = pid
            .parse()
            .map_err(|_| ApiError::bad("invalid project_id"))?;
        state
            .store
            .get_project(id)?
            .ok_or_else(|| ApiError::not_found("project not found"))?
    } else {
        state
            .store
            .get_or_create_project(&state.project_root, "default")?
    };

    let deadline = Some(
        chrono::Utc::now() + chrono::Duration::seconds(state.config.agent.timeout_seconds as i64),
    );
    let task = AgentTask::new(
        project.id,
        body.request,
        state.config.agent.max_iterations,
        state.config.agent.max_tool_calls,
        state.config.context.max_tokens.saturating_mul(4),
        state.config.context.max_tokens,
        deadline,
    );
    let id = task.id;
    state.store.create_task(&task)?;

    let cancel = CancellationToken::new();
    {
        let mut map = state.cancel_tokens.lock().await;
        map.insert(id, cancel.clone());
    }

    let store = state.store.clone();
    let tools = state.tools.clone();
    let router = state.router.clone();
    let config = state.config.clone();
    let root = state.project_root.clone();
    let slots = state.task_slots.clone();

    tokio::spawn(async move {
        let Ok(_permit) = slots.acquire().await else {
            return;
        };
        let orch = Orchestrator::with_router(store, tools, router, config, root);
        let _ = orch.run(id, cancel).await;
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(CreateTaskResponse {
            id: id.to_string(),
            phase: "created".into(),
        }),
    ))
}

async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let tid: TaskId = id.parse().map_err(|_| ApiError::bad("invalid task id"))?;
    let task = state
        .store
        .get_task(tid)?
        .ok_or_else(|| ApiError::not_found("task not found"))?;
    Ok(Json(task))
}

async fn cancel_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let tid: TaskId = id.parse().map_err(|_| ApiError::bad("invalid task id"))?;
    state.store.request_cancel(tid)?;
    if let Some(token) = state.cancel_tokens.lock().await.get(&tid) {
        token.cancel();
    }
    Ok(Json(serde_json::json!({"cancelled": true, "id": id})))
}

#[derive(Debug, Deserialize)]
pub struct ApproveRequest {
    pub call_id: String,
    #[serde(default = "default_granted")]
    pub granted: bool,
}

fn default_granted() -> bool {
    true
}

async fn approve_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ApproveRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let tid: TaskId = id.parse().map_err(|_| ApiError::bad("invalid task id"))?;
    let cid: ToolCallId = body
        .call_id
        .parse()
        .map_err(|_| ApiError::bad("invalid call_id"))?;
    let task = state
        .store
        .get_task(tid)?
        .ok_or_else(|| ApiError::not_found("task not found"))?;
    if task.phase != TaskPhase::WaitingApproval {
        return Err(ApiError::bad("task is not waiting for approval"));
    }
    let cp = state
        .store
        .load_checkpoint(tid)?
        .ok_or_else(|| ApiError::bad("no checkpoint for task"))?;
    let pending_id = cp.pending_call.as_ref().map(|c| c.id);
    if pending_id != Some(cid) {
        return Err(ApiError::bad("call_id does not match pending approval"));
    }
    state.store.set_approval(tid, cid, body.granted)?;

    let cancel = CancellationToken::new();
    {
        let mut map = state.cancel_tokens.lock().await;
        map.insert(tid, cancel.clone());
    }

    let store = state.store.clone();
    let tools = state.tools.clone();
    let router = state.router.clone();
    let config = state.config.clone();
    let root = state.project_root.clone();
    let slots = state.task_slots.clone();

    tokio::spawn(async move {
        let Ok(_permit) = slots.acquire().await else {
            return;
        };
        let orch = Orchestrator::with_router(store, tools, router, config, root);
        let _ = orch.run(tid, cancel).await;
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "id": id,
            "call_id": body.call_id,
            "granted": body.granted,
            "resumed": true,
        })),
    ))
}

#[derive(Debug, Deserialize)]
pub struct MemoryQuery {
    pub q: Option<String>,
    pub kind: Option<String>,
    #[serde(default = "default_memory_limit")]
    pub limit: u32,
}

fn default_memory_limit() -> u32 {
    20
}

async fn list_project_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<MemoryQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let pid: ProjectId = id
        .parse()
        .map_err(|_| ApiError::bad("invalid project id"))?;
    let _ = state
        .store
        .get_project(pid)?
        .ok_or_else(|| ApiError::not_found("project not found"))?;
    let rows = if let Some(q) = query.q.as_deref().filter(|s| !s.trim().is_empty()) {
        state.store.search_memories(pid, q, query.limit)?
    } else {
        let kind = match query.kind.as_deref() {
            Some(k) => Some(
                MemoryKind::parse(k).ok_or_else(|| ApiError::bad(format!("unknown kind: {k}")))?,
            ),
            None => None,
        };
        state.store.list_memories(pid, kind, query.limit)?
    };
    Ok(Json(rows))
}

async fn list_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let tid: TaskId = id.parse().map_err(|_| ApiError::bad("invalid task id"))?;
    let events = state.store.list_events(tid, None, 1000)?;
    let payload: Vec<_> = events
        .into_iter()
        .map(|(seq, e)| {
            serde_json::json!({
                "seq": seq,
                "id": e.id.to_string(),
                "kind": e.kind.as_str(),
                "evidence": e.evidence.as_str(),
                "payload": e.payload,
                "created_at": e.created_at,
            })
        })
        .collect();
    Ok(Json(payload))
}

async fn create_project(
    State(state): State<AppState>,
    Json(body): Json<CreateProjectRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let root = PathBuf::from(body.root_path);
    let project = state.store.get_or_create_project(&root, &body.name)?;
    Ok(Json(serde_json::json!({
        "id": project.id.to_string(),
        "root_path": project.root_path,
        "name": project.name,
    })))
}

async fn index_project(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let pid: ProjectId = id
        .parse()
        .map_err(|_| ApiError::bad("invalid project id"))?;
    let project = state
        .store
        .get_project(pid)?
        .ok_or_else(|| ApiError::not_found("project not found"))?;
    let stats = raya_index::index_project(&state.store, &project.root_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(serde_json::json!({
        "scanned": stats.scanned,
        "updated": stats.updated,
        "unchanged": stats.unchanged,
        "removed": stats.removed,
        "symbols": stats.symbols,
    })))
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }
    fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: msg.into(),
        }
    }
    fn internal(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: msg.into(),
        }
    }
}

impl From<raya_store::StoreError> for ApiError {
    fn from(e: raya_store::StoreError) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: e.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({"error": self.message})),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raya_llm::ModelRouter;
    use raya_policy::PolicyEngine;
    use raya_tools::default_registry;
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let store = Arc::new(Store::open(root.join("t.db")).unwrap());
        let config = Config::default();
        // Keep tempdir alive for the duration of the process in unit tests.
        std::mem::forget(dir);
        AppState {
            store,
            tools: Arc::new(default_registry(PolicyEngine::from_defaults())),
            router: Arc::new(ModelRouter::from_config(&config, CancellationToken::new()).unwrap()),
            config: config.clone(),
            project_root: root,
            started: Instant::now(),
            task_slots: Arc::new(Semaphore::new(2)),
            cancel_tokens: Arc::new(tokio::sync::Mutex::new(Default::default())),
        }
    }

    #[tokio::test]
    async fn health_ok() {
        let app = router(test_state());
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
