//! SQLite-backed store implementation.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use raya_core::{
    AgentTask, DagNode, DagNodeStatus, Event, EventKind, EvidenceKind, ExecutionPlan, MemoryKind,
    MemoryRecord, Message, ProjectId, TaskCheckpoint, TaskDagNodeRecord, TaskId, TaskPhase,
    ToolCall, ToolCallId, redact_secrets,
};
use rusqlite::{Connection, OptionalExtension, params};
use rusqlite_migration::{M, Migrations};
use serde::{Deserialize, Serialize};
use tracing::debug;
use uuid::Uuid;

use crate::error::{StoreError, StoreResult};

const MIGRATION_0001: &str = include_str!("../../../migrations/0001_init.sql");
const MIGRATION_0002: &str = include_str!("../../../migrations/0002_index.sql");
const MIGRATION_0003: &str = include_str!("../../../migrations/0003_memory_checkpoints.sql");
const MIGRATION_0004: &str = include_str!("../../../migrations/0004_task_dag.sql");

/// Persisted project row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: ProjectId,
    pub root_path: PathBuf,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Thread-safe SQLite store.
pub struct Store {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl Store {
    /// Open (or create) a SQLite database at `path` and run migrations.
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StoreError::Message(format!("failed to create db directory: {e}")))?;
        }

        let mut conn = Connection::open(&path)?;
        configure_connection(&conn)?;
        run_migrations(&mut conn)?;

        debug!(path = %path.display(), "store opened");
        Ok(Self {
            conn: Mutex::new(conn),
            path,
        })
    }

    /// Open an in-memory database (tests).
    pub fn open_in_memory() -> StoreResult<Self> {
        let mut conn = Connection::open_in_memory()?;
        configure_connection(&conn)?;
        run_migrations(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path: PathBuf::from(":memory:"),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> StoreResult<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| StoreError::Message("store mutex poisoned".into()))
    }

    pub fn create_project(&self, root_path: &Path, name: &str) -> StoreResult<ProjectRecord> {
        let id = ProjectId::new();
        let now = Utc::now();
        let root = root_path
            .canonicalize()
            .unwrap_or_else(|_| root_path.to_path_buf());
        let root_str = root.display().to_string();

        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO projects (id, root_path, name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id.to_string(),
                root_str,
                name,
                now.to_rfc3339(),
                now.to_rfc3339()
            ],
        )?;

        Ok(ProjectRecord {
            id,
            root_path: root,
            name: name.to_string(),
            created_at: now,
            updated_at: now,
        })
    }

    pub fn get_project(&self, id: ProjectId) -> StoreResult<Option<ProjectRecord>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT id, root_path, name, created_at, updated_at FROM projects WHERE id = ?1",
            params![id.to_string()],
            map_project,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn get_project_by_root(&self, root_path: &Path) -> StoreResult<Option<ProjectRecord>> {
        let root = root_path
            .canonicalize()
            .unwrap_or_else(|_| root_path.to_path_buf());
        let conn = self.lock()?;
        conn.query_row(
            "SELECT id, root_path, name, created_at, updated_at FROM projects WHERE root_path = ?1",
            params![root.display().to_string()],
            map_project,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn get_or_create_project(
        &self,
        root_path: &Path,
        name: &str,
    ) -> StoreResult<ProjectRecord> {
        if let Some(p) = self.get_project_by_root(root_path)? {
            return Ok(p);
        }
        self.create_project(root_path, name)
    }

    pub fn create_task(&self, task: &AgentTask) -> StoreResult<()> {
        let conn = self.lock()?;
        let plan_json = match &task.plan {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        conn.execute(
            "INSERT INTO tasks (
                id, project_id, request, phase, plan_json, current_step,
                max_iterations, max_tool_calls, max_tokens, context_token_budget,
                deadline, iterations, tool_calls, tokens_used, error,
                cancel_requested, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
             )",
            params![
                task.id.to_string(),
                task.project_id.to_string(),
                task.request,
                phase_str(task.phase),
                plan_json,
                task.current_step,
                task.max_iterations,
                task.max_tool_calls,
                task.max_tokens as i64,
                task.context_token_budget as i64,
                task.deadline.map(|d| d.to_rfc3339()),
                task.iterations,
                task.tool_calls,
                task.tokens_used as i64,
                task.error,
                i32::from(task.cancel_requested),
                task.created_at.to_rfc3339(),
                task.updated_at.to_rfc3339(),
            ],
        )?;

        let event = Event::new(
            task.id,
            EventKind::TaskCreated,
            serde_json::json!({ "request": task.request }),
        );
        insert_event(&conn, &event)?;
        Ok(())
    }

    pub fn get_task(&self, id: TaskId) -> StoreResult<Option<AgentTask>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT id, project_id, request, phase, plan_json, current_step,
                    max_iterations, max_tool_calls, max_tokens, context_token_budget,
                    deadline, iterations, tool_calls, tokens_used, error,
                    cancel_requested, created_at, updated_at
             FROM tasks WHERE id = ?1",
            params![id.to_string()],
            map_task,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_tasks(
        &self,
        project_id: Option<ProjectId>,
        limit: u32,
    ) -> StoreResult<Vec<AgentTask>> {
        let conn = self.lock()?;
        let limit = limit as i64;
        let mut tasks = Vec::new();

        if let Some(pid) = project_id {
            let mut stmt = conn.prepare(
                "SELECT id, project_id, request, phase, plan_json, current_step,
                        max_iterations, max_tool_calls, max_tokens, context_token_budget,
                        deadline, iterations, tool_calls, tokens_used, error,
                        cancel_requested, created_at, updated_at
                 FROM tasks WHERE project_id = ?1
                 ORDER BY updated_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![pid.to_string(), limit], map_task)?;
            for row in rows {
                tasks.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, project_id, request, phase, plan_json, current_step,
                        max_iterations, max_tool_calls, max_tokens, context_token_budget,
                        deadline, iterations, tool_calls, tokens_used, error,
                        cancel_requested, created_at, updated_at
                 FROM tasks ORDER BY updated_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], map_task)?;
            for row in rows {
                tasks.push(row?);
            }
        }
        Ok(tasks)
    }

    /// Atomically transition a task phase and append a phase-changed event.
    pub fn transition_task(
        &self,
        id: TaskId,
        from: TaskPhase,
        to: TaskPhase,
    ) -> StoreResult<AgentTask> {
        if !from.can_transition_to(to) {
            return Err(StoreError::InvalidTransition { from, to });
        }

        let conn = self.lock()?;
        let tx = conn.unchecked_transaction()?;

        let mut task = tx
            .query_row(
                "SELECT id, project_id, request, phase, plan_json, current_step,
                        max_iterations, max_tool_calls, max_tokens, context_token_budget,
                        deadline, iterations, tool_calls, tokens_used, error,
                        cancel_requested, created_at, updated_at
                 FROM tasks WHERE id = ?1",
                params![id.to_string()],
                map_task,
            )
            .optional()?
            .ok_or_else(|| StoreError::TaskNotFound(id.to_string()))?;

        if task.phase != from {
            return Err(StoreError::Message(format!(
                "expected phase {from:?}, found {:?}",
                task.phase
            )));
        }

        let now = Utc::now();
        tx.execute(
            "UPDATE tasks SET phase = ?1, updated_at = ?2 WHERE id = ?3",
            params![phase_str(to), now.to_rfc3339(), id.to_string()],
        )?;

        let event = Event::new(
            id,
            EventKind::PhaseChanged,
            serde_json::json!({
                "from": phase_str(from),
                "to": phase_str(to),
            }),
        );
        insert_event(&tx, &event)?;

        // Terminal event kinds
        let terminal = match to {
            TaskPhase::Completed => Some(EventKind::TaskCompleted),
            TaskPhase::Failed => Some(EventKind::TaskFailed),
            TaskPhase::Cancelled => Some(EventKind::TaskCancelled),
            _ => None,
        };
        if let Some(kind) = terminal {
            let ev = Event::new(id, kind, serde_json::json!({}));
            insert_event(&tx, &ev)?;
        }

        tx.commit()?;
        task.phase = to;
        task.updated_at = now;
        Ok(task)
    }

    pub fn update_task(&self, task: &AgentTask) -> StoreResult<()> {
        let conn = self.lock()?;
        let plan_json = match &task.plan {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let n = conn.execute(
            "UPDATE tasks SET
                request = ?1, phase = ?2, plan_json = ?3, current_step = ?4,
                iterations = ?5, tool_calls = ?6, tokens_used = ?7, error = ?8,
                cancel_requested = ?9, updated_at = ?10
             WHERE id = ?11",
            params![
                task.request,
                phase_str(task.phase),
                plan_json,
                task.current_step,
                task.iterations,
                task.tool_calls,
                task.tokens_used as i64,
                task.error,
                i32::from(task.cancel_requested),
                Utc::now().to_rfc3339(),
                task.id.to_string(),
            ],
        )?;
        if n == 0 {
            return Err(StoreError::TaskNotFound(task.id.to_string()));
        }
        Ok(())
    }

    pub fn append_event(&self, event: &Event) -> StoreResult<()> {
        let conn = self.lock()?;
        insert_event(&conn, event)
    }

    pub fn list_events(
        &self,
        task_id: TaskId,
        after_seq: Option<i64>,
        limit: u32,
    ) -> StoreResult<Vec<(i64, Event)>> {
        let conn = self.lock()?;
        let after = after_seq.unwrap_or(0);
        let mut stmt = conn.prepare(
            "SELECT id, task_id, kind, payload_json, created_at, seq, evidence
             FROM events
             WHERE task_id = ?1 AND seq > ?2
             ORDER BY seq ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![task_id.to_string(), after, limit as i64], |row| {
            let seq: i64 = row.get(5)?;
            let event = map_event_row(row)?;
            Ok((seq, event))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn request_cancel(&self, id: TaskId) -> StoreResult<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "UPDATE tasks SET cancel_requested = 1, updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id.to_string()],
        )?;
        Ok(n > 0)
    }

    pub fn is_cancel_requested(&self, id: TaskId) -> StoreResult<bool> {
        let conn = self.lock()?;
        let flag: i32 = conn
            .query_row(
                "SELECT cancel_requested FROM tasks WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::TaskNotFound(id.to_string()))?;
        Ok(flag != 0)
    }

    pub fn set_approval(
        &self,
        task_id: TaskId,
        call_id: ToolCallId,
        granted: bool,
    ) -> StoreResult<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO approvals (call_id, task_id, granted, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(call_id) DO UPDATE SET granted = excluded.granted",
            params![
                call_id.to_string(),
                task_id.to_string(),
                i32::from(granted),
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn is_approved(&self, call_id: ToolCallId) -> StoreResult<bool> {
        Ok(self.approval_status(call_id)? == Some(true))
    }

    /// `None` = pending / no row; `Some(true)` = granted; `Some(false)` = denied.
    pub fn approval_status(&self, call_id: ToolCallId) -> StoreResult<Option<bool>> {
        let conn = self.lock()?;
        let granted: Option<i32> = conn
            .query_row(
                "SELECT granted FROM approvals WHERE call_id = ?1",
                params![call_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(granted.map(|g| g != 0))
    }

    pub fn save_checkpoint(&self, cp: &TaskCheckpoint) -> StoreResult<()> {
        let conn = self.lock()?;
        let pending = match &cp.pending_call {
            Some(call) => Some(
                serde_json::to_string(call)
                    .map_err(|e| StoreError::Message(format!("serialize pending_call: {e}")))?,
            ),
            None => None,
        };
        let messages = serde_json::to_string(&cp.messages)
            .map_err(|e| StoreError::Message(format!("serialize messages: {e}")))?;
        conn.execute(
            "INSERT INTO task_checkpoints (task_id, pending_call_json, messages_json, review_rounds, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(task_id) DO UPDATE SET
               pending_call_json = excluded.pending_call_json,
               messages_json = excluded.messages_json,
               review_rounds = excluded.review_rounds,
               updated_at = excluded.updated_at",
            params![
                cp.task_id.to_string(),
                pending,
                messages,
                cp.review_rounds as i64,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn load_checkpoint(&self, task_id: TaskId) -> StoreResult<Option<TaskCheckpoint>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT task_id, pending_call_json, messages_json, review_rounds
             FROM task_checkpoints WHERE task_id = ?1",
            params![task_id.to_string()],
            |row| {
                let tid: String = row.get(0)?;
                let pending_json: Option<String> = row.get(1)?;
                let messages_json: String = row.get(2)?;
                let review_rounds: i64 = row.get(3)?;
                let pending_call = match pending_json {
                    Some(s) if !s.is_empty() => {
                        Some(serde_json::from_str::<ToolCall>(&s).map_err(|e| {
                            rusqlite::Error::FromSqlConversionFailure(
                                1,
                                rusqlite::types::Type::Text,
                                Box::new(e),
                            )
                        })?)
                    }
                    _ => None,
                };
                let messages: Vec<Message> = serde_json::from_str(&messages_json).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                Ok(TaskCheckpoint {
                    task_id: TaskId::from_uuid(Uuid::parse_str(&tid).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?),
                    pending_call,
                    messages,
                    review_rounds: review_rounds as u32,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn delete_checkpoint(&self, task_id: TaskId) -> StoreResult<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "DELETE FROM task_checkpoints WHERE task_id = ?1",
            params![task_id.to_string()],
        )?;
        Ok(n > 0)
    }

    /// Replace all DAG nodes for a task (status = pending).
    pub fn replace_dag(&self, task_id: TaskId, nodes: &[DagNode]) -> StoreResult<()> {
        let conn = self.lock()?;
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM task_dag_nodes WHERE task_id = ?1",
            params![task_id.to_string()],
        )?;
        let now = Utc::now().to_rfc3339();
        for n in nodes {
            let paths = serde_json::to_string(&n.paths)
                .map_err(|e| StoreError::Message(format!("serialize paths: {e}")))?;
            let deps = serde_json::to_string(&n.depends_on)
                .map_err(|e| StoreError::Message(format!("serialize depends_on: {e}")))?;
            tx.execute(
                "INSERT INTO task_dag_nodes
                 (task_id, node_id, role, objective, paths_json, depends_on_json, status, summary, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8)",
                params![
                    task_id.to_string(),
                    n.id,
                    n.role,
                    n.objective,
                    paths,
                    deps,
                    DagNodeStatus::Pending.as_str(),
                    now,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_dag_nodes(&self, task_id: TaskId) -> StoreResult<Vec<TaskDagNodeRecord>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT task_id, node_id, role, objective, paths_json, depends_on_json, status, summary, updated_at
             FROM task_dag_nodes WHERE task_id = ?1 ORDER BY node_id",
        )?;
        let rows = stmt.query_map(params![task_id.to_string()], map_dag_node)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn set_dag_node_status(
        &self,
        task_id: TaskId,
        node_id: &str,
        status: DagNodeStatus,
        summary: Option<&str>,
    ) -> StoreResult<bool> {
        let conn = self.lock()?;
        let summary = summary.map(redact_secrets);
        let n = conn.execute(
            "UPDATE task_dag_nodes SET status = ?1, summary = ?2, updated_at = ?3
             WHERE task_id = ?4 AND node_id = ?5",
            params![
                status.as_str(),
                summary,
                Utc::now().to_rfc3339(),
                task_id.to_string(),
                node_id,
            ],
        )?;
        Ok(n > 0)
    }

    pub fn delete_dag(&self, task_id: TaskId) -> StoreResult<bool> {
        let conn = self.lock()?;
        let n = conn.execute(
            "DELETE FROM task_dag_nodes WHERE task_id = ?1",
            params![task_id.to_string()],
        )?;
        Ok(n > 0)
    }

    /// Reset nodes stuck in `running` (dead process) back to `pending` for crash-resume.
    pub fn reset_running_dag_nodes(&self, task_id: TaskId) -> StoreResult<u32> {
        let conn = self.lock()?;
        let n = conn.execute(
            "UPDATE task_dag_nodes SET status = ?1, summary = NULL, updated_at = ?2
             WHERE task_id = ?3 AND status = ?4",
            params![
                DagNodeStatus::Pending.as_str(),
                Utc::now().to_rfc3339(),
                task_id.to_string(),
                DagNodeStatus::Running.as_str(),
            ],
        )?;
        Ok(n as u32)
    }

    /// True if any DAG node for the task is still `pending` or `running`.
    pub fn has_incomplete_dag(&self, task_id: TaskId) -> StoreResult<bool> {
        let conn = self.lock()?;
        let found: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM task_dag_nodes
             WHERE task_id = ?1 AND status IN (?2, ?3)
             LIMIT 1",
                params![
                    task_id.to_string(),
                    DagNodeStatus::Pending.as_str(),
                    DagNodeStatus::Running.as_str(),
                ],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    pub fn upsert_memory(&self, mem: &MemoryRecord) -> StoreResult<()> {
        let conn = self.lock()?;
        let tags = serde_json::to_string(&mem.tags)
            .map_err(|e| StoreError::Message(format!("serialize tags: {e}")))?;
        let task_id = mem.task_id.map(|t| t.to_string());
        conn.execute(
            "INSERT INTO memories (id, project_id, task_id, kind, key, content, tags_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
               project_id = excluded.project_id,
               task_id = excluded.task_id,
               kind = excluded.kind,
               key = excluded.key,
               content = excluded.content,
               tags_json = excluded.tags_json,
               updated_at = excluded.updated_at",
            params![
                mem.id,
                mem.project_id.to_string(),
                task_id,
                mem.kind.as_str(),
                mem.key,
                mem.content,
                tags,
                mem.created_at.to_rfc3339(),
                mem.updated_at.to_rfc3339()
            ],
        )?;
        conn.execute(
            "DELETE FROM memories_fts WHERE memory_id = ?1",
            params![mem.id],
        )?;
        conn.execute(
            "INSERT INTO memories_fts (memory_id, content) VALUES (?1, ?2)",
            params![mem.id, mem.content],
        )?;
        Ok(())
    }

    pub fn list_memories(
        &self,
        project_id: ProjectId,
        kind: Option<MemoryKind>,
        limit: u32,
    ) -> StoreResult<Vec<MemoryRecord>> {
        let conn = self.lock()?;
        let limit = limit.clamp(1, 100) as i64;
        let mut out = Vec::new();
        if let Some(k) = kind {
            let mut stmt = conn.prepare(
                "SELECT id, project_id, task_id, kind, key, content, tags_json, created_at, updated_at
                 FROM memories
                 WHERE project_id = ?1 AND kind = ?2
                 ORDER BY updated_at DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(
                params![project_id.to_string(), k.as_str(), limit],
                map_memory,
            )?;
            for row in rows {
                out.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, project_id, task_id, kind, key, content, tags_json, created_at, updated_at
                 FROM memories
                 WHERE project_id = ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![project_id.to_string(), limit], map_memory)?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    pub fn search_memories(
        &self,
        project_id: ProjectId,
        query: &str,
        limit: u32,
    ) -> StoreResult<Vec<MemoryRecord>> {
        let conn = self.lock()?;
        let limit = limit.clamp(1, 100) as i64;
        let q = fts_query(query);
        let mut out = Vec::new();
        if q.is_empty() {
            return self.list_memories(project_id, None, limit as u32);
        }
        let mut stmt = conn.prepare(
            "SELECT m.id, m.project_id, m.task_id, m.kind, m.key, m.content, m.tags_json, m.created_at, m.updated_at
             FROM memories_fts
             JOIN memories m ON m.id = memories_fts.memory_id
             WHERE m.project_id = ?1 AND memories_fts MATCH ?2
             ORDER BY m.updated_at DESC
             LIMIT ?3",
        )?;
        if let Ok(rows) = stmt.query_map(params![project_id.to_string(), q, limit], map_memory) {
            for row in rows {
                out.push(row?);
            }
        }
        if out.is_empty() {
            let like = format!("%{}%", query.trim());
            let mut stmt = conn.prepare(
                "SELECT id, project_id, task_id, kind, key, content, tags_json, created_at, updated_at
                 FROM memories
                 WHERE project_id = ?1 AND content LIKE ?2
                 ORDER BY updated_at DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![project_id.to_string(), like, limit], map_memory)?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    pub fn delete_memory(&self, id: &str) -> StoreResult<bool> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM memories_fts WHERE memory_id = ?1", params![id])?;
        let n = conn.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    pub fn set_index_meta(&self, key: &str, value: &str) -> StoreResult<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO index_meta (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn get_index_meta(&self, key: &str) -> StoreResult<Option<String>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT value FROM index_meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    /// Run a closure with exclusive access to the SQLite connection.
    pub fn with_conn<F, T>(&self, f: F) -> StoreResult<T>
    where
        F: FnOnce(&Connection) -> StoreResult<T>,
    {
        let conn = self.lock()?;
        f(&conn)
    }
}

fn configure_connection(conn: &Connection) -> StoreResult<()> {
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(())
}

fn run_migrations(conn: &mut Connection) -> StoreResult<()> {
    let strip = |sql: &str| {
        sql.lines()
            .filter(|l| !l.trim().starts_with("PRAGMA"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let m1 = strip(MIGRATION_0001);
    let m2 = strip(MIGRATION_0002);
    let m3 = strip(MIGRATION_0003);
    let m4 = strip(MIGRATION_0004);
    let migrations = Migrations::new(vec![M::up(&m1), M::up(&m2), M::up(&m3), M::up(&m4)]);
    migrations.to_latest(conn)?;
    Ok(())
}

fn phase_str(phase: TaskPhase) -> &'static str {
    match phase {
        TaskPhase::Created => "created",
        TaskPhase::Planning => "planning",
        TaskPhase::ContextBuilding => "context_building",
        TaskPhase::Executing => "executing",
        TaskPhase::Verifying => "verifying",
        TaskPhase::Fixing => "fixing",
        TaskPhase::WaitingApproval => "waiting_approval",
        TaskPhase::Completed => "completed",
        TaskPhase::Failed => "failed",
        TaskPhase::Cancelled => "cancelled",
    }
}

fn parse_phase(s: &str) -> StoreResult<TaskPhase> {
    Ok(match s {
        "created" => TaskPhase::Created,
        "planning" => TaskPhase::Planning,
        "context_building" => TaskPhase::ContextBuilding,
        "executing" => TaskPhase::Executing,
        "verifying" => TaskPhase::Verifying,
        "fixing" => TaskPhase::Fixing,
        "waiting_approval" => TaskPhase::WaitingApproval,
        "completed" => TaskPhase::Completed,
        "failed" => TaskPhase::Failed,
        "cancelled" => TaskPhase::Cancelled,
        other => return Err(StoreError::InvalidPhase(other.to_string())),
    })
}

fn map_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRecord> {
    let id: String = row.get(0)?;
    let root: String = row.get(1)?;
    let name: String = row.get(2)?;
    let created: String = row.get(3)?;
    let updated: String = row.get(4)?;
    Ok(ProjectRecord {
        id: ProjectId::from_uuid(Uuid::parse_str(&id).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?),
        root_path: PathBuf::from(root),
        name,
        created_at: parse_dt(&created)?,
        updated_at: parse_dt(&updated)?,
    })
}

fn map_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentTask> {
    let id: String = row.get(0)?;
    let project_id: String = row.get(1)?;
    let request: String = row.get(2)?;
    let phase: String = row.get(3)?;
    let plan_json: Option<String> = row.get(4)?;
    let current_step: Option<String> = row.get(5)?;
    let max_iterations: u32 = row.get(6)?;
    let max_tool_calls: u32 = row.get(7)?;
    let max_tokens: i64 = row.get(8)?;
    let context_token_budget: i64 = row.get(9)?;
    let deadline: Option<String> = row.get(10)?;
    let iterations: u32 = row.get(11)?;
    let tool_calls: u32 = row.get(12)?;
    let tokens_used: i64 = row.get(13)?;
    let error: Option<String> = row.get(14)?;
    let cancel_requested: i32 = row.get(15)?;
    let created_at: String = row.get(16)?;
    let updated_at: String = row.get(17)?;

    let plan: Option<ExecutionPlan> = match plan_json {
        Some(j) => Some(serde_json::from_str(&j).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?),
        None => None,
    };

    Ok(AgentTask {
        id: TaskId::from_uuid(parse_uuid(&id, 0)?),
        project_id: ProjectId::from_uuid(parse_uuid(&project_id, 1)?),
        request,
        phase: parse_phase(&phase).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::other(e.to_string())),
            )
        })?,
        plan,
        current_step,
        max_iterations,
        max_tool_calls,
        max_tokens: max_tokens as u64,
        context_token_budget: context_token_budget as u64,
        deadline: deadline.as_deref().map(parse_dt).transpose()?,
        iterations,
        tool_calls,
        tokens_used: tokens_used as u64,
        error,
        cancel_requested: cancel_requested != 0,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
    })
}

fn map_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    let id: String = row.get(0)?;
    let task_id: String = row.get(1)?;
    let kind: String = row.get(2)?;
    let payload: String = row.get(3)?;
    let created_at: String = row.get(4)?;
    let evidence_str: String = row.get(6)?;
    Ok(Event {
        id: raya_core::EventId::from_uuid(parse_uuid(&id, 0)?),
        task_id: TaskId::from_uuid(parse_uuid(&task_id, 1)?),
        kind: EventKind::parse(&kind).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::other(format!("bad event kind {kind}"))),
            )
        })?,
        evidence: EvidenceKind::parse(&evidence_str).unwrap_or(EvidenceKind::Unknown),
        payload: serde_json::from_str(&payload).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?,
        created_at: parse_dt(&created_at)?,
    })
}

fn insert_event(conn: &Connection, event: &Event) -> StoreResult<()> {
    let next_seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE task_id = ?1",
        params![event.task_id.to_string()],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO events (id, task_id, kind, payload_json, created_at, seq, evidence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            event.id.to_string(),
            event.task_id.to_string(),
            event.kind.as_str(),
            serde_json::to_string(&event.payload)?,
            event.created_at.to_rfc3339(),
            next_seq,
            event.evidence.as_str(),
        ],
    )?;
    Ok(())
}

fn map_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRecord> {
    let id: String = row.get(0)?;
    let project_id: String = row.get(1)?;
    let task_id: Option<String> = row.get(2)?;
    let kind: String = row.get(3)?;
    let key: Option<String> = row.get(4)?;
    let content: String = row.get(5)?;
    let tags_json: String = row.get(6)?;
    let created_at: String = row.get(7)?;
    let updated_at: String = row.get(8)?;
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
    Ok(MemoryRecord {
        id,
        project_id: ProjectId::from_uuid(parse_uuid(&project_id, 1)?),
        task_id: match task_id.as_deref() {
            Some(s) => Some(TaskId::from_uuid(parse_uuid(s, 2)?)),
            None => None,
        },
        kind: MemoryKind::parse(&kind).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::other(format!("bad memory kind {kind}"))),
            )
        })?,
        key,
        content,
        tags,
        created_at: parse_dt(&created_at)?,
        updated_at: parse_dt(&updated_at)?,
    })
}

fn map_dag_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskDagNodeRecord> {
    let task_id: String = row.get(0)?;
    let node_id: String = row.get(1)?;
    let role: String = row.get(2)?;
    let objective: String = row.get(3)?;
    let paths_json: String = row.get(4)?;
    let depends_on_json: String = row.get(5)?;
    let status: String = row.get(6)?;
    let summary: Option<String> = row.get(7)?;
    let updated_at: String = row.get(8)?;
    let paths: Vec<String> = serde_json::from_str(&paths_json).unwrap_or_default();
    let depends_on: Vec<String> = serde_json::from_str(&depends_on_json).unwrap_or_default();
    let status = DagNodeStatus::parse(&status).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(format!("bad dag status {status}"))),
        )
    })?;
    Ok(TaskDagNodeRecord {
        task_id: TaskId::from_uuid(parse_uuid(&task_id, 0)?),
        node_id,
        role,
        objective,
        paths,
        depends_on,
        status,
        summary,
        updated_at: parse_dt(&updated_at)?,
    })
}

/// Build a loose FTS5 MATCH query from free text.
fn fts_query(query: &str) -> String {
    query
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
        .map(|t| {
            let clean: String = t
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if clean.is_empty() {
                String::new()
            } else {
                format!("\"{clean}\"")
            }
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn parse_uuid(s: &str, idx: usize) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn parse_dt(s: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use raya_core::AgentTask;
    use tempfile::tempdir;

    fn sample_task(project_id: ProjectId) -> AgentTask {
        AgentTask::new(project_id, "do the thing", 12, 40, 100_000, 40_000, None)
    }

    #[test]
    fn create_and_get_task() {
        let store = Store::open_in_memory().unwrap();
        let project = store
            .create_project(Path::new("/tmp/proj"), "proj")
            .unwrap();
        let task = sample_task(project.id);
        store.create_task(&task).unwrap();
        let loaded = store.get_task(task.id).unwrap().unwrap();
        assert_eq!(loaded.request, "do the thing");
        assert_eq!(loaded.phase, TaskPhase::Created);
    }

    #[test]
    fn transition_persists_events() {
        let store = Store::open_in_memory().unwrap();
        let project = store
            .create_project(Path::new("/tmp/proj2"), "proj")
            .unwrap();
        let task = sample_task(project.id);
        store.create_task(&task).unwrap();
        store
            .transition_task(task.id, TaskPhase::Created, TaskPhase::Planning)
            .unwrap();
        let events = store.list_events(task.id, None, 100).unwrap();
        assert!(events.len() >= 2);
        assert_eq!(events[0].1.kind, EventKind::TaskCreated);
        assert_eq!(events[1].1.kind, EventKind::PhaseChanged);
    }

    #[test]
    fn invalid_transition_rejected() {
        let store = Store::open_in_memory().unwrap();
        let project = store
            .create_project(Path::new("/tmp/proj3"), "proj")
            .unwrap();
        let task = sample_task(project.id);
        store.create_task(&task).unwrap();
        let err = store
            .transition_task(task.id, TaskPhase::Created, TaskPhase::Executing)
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidTransition { .. }));
    }

    #[test]
    fn cancel_flag() {
        let store = Store::open_in_memory().unwrap();
        let project = store
            .create_project(Path::new("/tmp/proj4"), "proj")
            .unwrap();
        let task = sample_task(project.id);
        store.create_task(&task).unwrap();
        assert!(!store.is_cancel_requested(task.id).unwrap());
        assert!(store.request_cancel(task.id).unwrap());
        assert!(store.is_cancel_requested(task.id).unwrap());
    }

    #[test]
    fn reopen_persists() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("raya.db");
        let id;
        {
            let store = Store::open(&path).unwrap();
            let project = store.create_project(dir.path(), "proj").unwrap();
            let task = sample_task(project.id);
            id = task.id;
            store.create_task(&task).unwrap();
        }
        let store = Store::open(&path).unwrap();
        let loaded = store.get_task(id).unwrap().unwrap();
        assert_eq!(loaded.id, id);
    }

    #[test]
    fn event_ordering() {
        let store = Store::open_in_memory().unwrap();
        let project = store
            .create_project(Path::new("/tmp/proj5"), "proj")
            .unwrap();
        let task = sample_task(project.id);
        store.create_task(&task).unwrap();
        for i in 0..5 {
            store
                .append_event(&Event::new(
                    task.id,
                    EventKind::LlmRequest,
                    serde_json::json!({ "i": i }),
                ))
                .unwrap();
        }
        let events = store.list_events(task.id, Some(0), 100).unwrap();
        let seqs: Vec<i64> = events.iter().map(|(s, _)| *s).collect();
        let mut sorted = seqs.clone();
        sorted.sort_unstable();
        assert_eq!(seqs, sorted);
    }

    #[test]
    fn checkpoint_roundtrip() {
        use raya_core::{Message, TaskCheckpoint, ToolCall};
        let store = Store::open_in_memory().unwrap();
        let project = store.create_project(Path::new("/tmp/cp"), "proj").unwrap();
        let task = sample_task(project.id);
        store.create_task(&task).unwrap();
        let call = ToolCall::new("shell.exec", serde_json::json!({"command": "echo hi"}));
        let cp = TaskCheckpoint {
            task_id: task.id,
            pending_call: Some(call.clone()),
            messages: vec![Message::user("hi"), Message::assistant("ok")],
            review_rounds: 1,
        };
        store.save_checkpoint(&cp).unwrap();
        let loaded = store.load_checkpoint(task.id).unwrap().unwrap();
        assert_eq!(loaded.review_rounds, 1);
        assert_eq!(loaded.pending_call.as_ref().unwrap().name, "shell.exec");
        assert_eq!(loaded.messages.len(), 2);
        assert!(store.delete_checkpoint(task.id).unwrap());
        assert!(store.load_checkpoint(task.id).unwrap().is_none());
    }

    #[test]
    fn approval_status_three_states() {
        let store = Store::open_in_memory().unwrap();
        let project = store
            .create_project(Path::new("/tmp/appr"), "proj")
            .unwrap();
        let task = sample_task(project.id);
        store.create_task(&task).unwrap();
        let call_id = ToolCallId::new();
        assert_eq!(store.approval_status(call_id).unwrap(), None);
        store.set_approval(task.id, call_id, true).unwrap();
        assert_eq!(store.approval_status(call_id).unwrap(), Some(true));
        store.set_approval(task.id, call_id, false).unwrap();
        assert_eq!(store.approval_status(call_id).unwrap(), Some(false));
        assert!(!store.is_approved(call_id).unwrap());
    }

    #[test]
    fn memory_upsert_list_search_delete() {
        use raya_core::{MemoryKind, MemoryRecord};
        let dir = tempdir().unwrap();
        let path = dir.path().join("mem.db");
        let project_id;
        let mem_id;
        {
            let store = Store::open(&path).unwrap();
            let project = store.create_project(dir.path(), "proj").unwrap();
            project_id = project.id;
            let task = sample_task(project.id);
            store.create_task(&task).unwrap();
            let mem = MemoryRecord::new(
                project.id,
                MemoryKind::Task,
                "wrote hello.txt successfully for demo",
                Some(task.id),
            )
            .with_key("summary");
            mem_id = mem.id.clone();
            store.upsert_memory(&mem).unwrap();
            let listed = store
                .list_memories(project.id, Some(MemoryKind::Task), 10)
                .unwrap();
            assert_eq!(listed.len(), 1);
            let hits = store.search_memories(project.id, "hello.txt", 10).unwrap();
            assert!(!hits.is_empty());
            assert!(hits.iter().any(|m| m.id == mem_id));
        }
        let store = Store::open(&path).unwrap();
        let listed = store.list_memories(project_id, None, 10).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(store.delete_memory(&mem_id).unwrap());
        assert!(
            store
                .list_memories(project_id, None, 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn dag_replace_list_status_delete() {
        use raya_core::DagNode;
        let dir = tempdir().unwrap();
        let path = dir.path().join("dag.db");
        let task_id;
        {
            let store = Store::open(&path).unwrap();
            let project = store.create_project(dir.path(), "proj").unwrap();
            let task = sample_task(project.id);
            task_id = task.id;
            store.create_task(&task).unwrap();
            let nodes = vec![
                DagNode {
                    id: "a".into(),
                    role: "coder".into(),
                    objective: "write A".into(),
                    paths: vec!["a.txt".into()],
                    depends_on: vec![],
                },
                DagNode {
                    id: "b".into(),
                    role: "coder".into(),
                    objective: "write B".into(),
                    paths: vec![],
                    depends_on: vec!["a".into()],
                },
            ];
            store.replace_dag(task.id, &nodes).unwrap();
            let listed = store.list_dag_nodes(task.id).unwrap();
            assert_eq!(listed.len(), 2);
            assert_eq!(listed[0].status, DagNodeStatus::Pending);
            assert!(
                store
                    .set_dag_node_status(task.id, "a", DagNodeStatus::Running, None)
                    .unwrap()
            );
            assert!(
                store
                    .set_dag_node_status(task.id, "a", DagNodeStatus::Completed, Some("done A"))
                    .unwrap()
            );
            let listed = store.list_dag_nodes(task.id).unwrap();
            let a = listed.iter().find(|n| n.node_id == "a").unwrap();
            assert_eq!(a.status, DagNodeStatus::Completed);
            assert_eq!(a.summary.as_deref(), Some("done A"));
            assert_eq!(a.depends_on, Vec::<String>::new());
            let b = listed.iter().find(|n| n.node_id == "b").unwrap();
            assert_eq!(b.depends_on, vec!["a".to_string()]);
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.list_dag_nodes(task_id).unwrap().len(), 2);
        assert!(store.delete_dag(task_id).unwrap());
        assert!(store.list_dag_nodes(task_id).unwrap().is_empty());
    }

    #[test]
    fn dag_reset_running_and_has_incomplete() {
        use raya_core::DagNode;
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path().join("dag-resume.db")).unwrap();
        let project = store.create_project(dir.path(), "proj").unwrap();
        let task = sample_task(project.id);
        store.create_task(&task).unwrap();
        let nodes = vec![
            DagNode {
                id: "a".into(),
                role: "coder".into(),
                objective: "write A".into(),
                paths: vec![],
                depends_on: vec![],
            },
            DagNode {
                id: "b".into(),
                role: "coder".into(),
                objective: "write B".into(),
                paths: vec![],
                depends_on: vec!["a".into()],
            },
        ];
        store.replace_dag(task.id, &nodes).unwrap();
        assert!(store.has_incomplete_dag(task.id).unwrap());

        store
            .set_dag_node_status(task.id, "a", DagNodeStatus::Running, Some("in progress"))
            .unwrap();
        store
            .set_dag_node_status(task.id, "b", DagNodeStatus::Completed, Some("done"))
            .unwrap();

        let reset = store.reset_running_dag_nodes(task.id).unwrap();
        assert_eq!(reset, 1);

        let listed = store.list_dag_nodes(task.id).unwrap();
        let a = listed.iter().find(|n| n.node_id == "a").unwrap();
        assert_eq!(a.status, DagNodeStatus::Pending);
        assert!(a.summary.is_none());
        let b = listed.iter().find(|n| n.node_id == "b").unwrap();
        assert_eq!(b.status, DagNodeStatus::Completed);
        assert_eq!(b.summary.as_deref(), Some("done"));

        assert!(store.has_incomplete_dag(task.id).unwrap());
        store
            .set_dag_node_status(task.id, "a", DagNodeStatus::Completed, Some("done A"))
            .unwrap();
        assert!(!store.has_incomplete_dag(task.id).unwrap());
        assert_eq!(store.reset_running_dag_nodes(task.id).unwrap(), 0);
    }
}
