//! SQLite-backed store implementation.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use raya_core::{
    AgentTask, Event, EventKind, EvidenceKind, ExecutionPlan, ProjectId, TaskId, TaskPhase,
    ToolCallId,
};
use rusqlite::{Connection, OptionalExtension, params};
use rusqlite_migration::{M, Migrations};
use serde::{Deserialize, Serialize};
use tracing::debug;
use uuid::Uuid;

use crate::error::{StoreError, StoreResult};

const MIGRATION_0001: &str = include_str!("../../../migrations/0001_init.sql");
const MIGRATION_0002: &str = include_str!("../../../migrations/0002_index.sql");

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
        let conn = self.lock()?;
        let granted: Option<i32> = conn
            .query_row(
                "SELECT granted FROM approvals WHERE call_id = ?1",
                params![call_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(granted == Some(1))
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
    let migrations = Migrations::new(vec![M::up(&m1), M::up(&m2)]);
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
}
