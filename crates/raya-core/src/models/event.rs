//! Event store models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ids::{EventId, TaskId};

/// Kind of structured event emitted by the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    TaskCreated,
    TaskStarted,
    PlanCreated,
    ContextRetrieved,
    LlmRequest,
    LlmResponse,
    ToolStarted,
    ToolCompleted,
    FileModified,
    TestStarted,
    TestFailed,
    TestPassed,
    AgentSpawned,
    AgentCompleted,
    ApprovalRequested,
    ApprovalGranted,
    ApprovalDenied,
    TaskCompleted,
    TaskFailed,
    TaskCancelled,
    PhaseChanged,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TaskCreated => "task.created",
            Self::TaskStarted => "task.started",
            Self::PlanCreated => "plan.created",
            Self::ContextRetrieved => "context.retrieved",
            Self::LlmRequest => "llm.request",
            Self::LlmResponse => "llm.response",
            Self::ToolStarted => "tool.started",
            Self::ToolCompleted => "tool.completed",
            Self::FileModified => "file.modified",
            Self::TestStarted => "test.started",
            Self::TestFailed => "test.failed",
            Self::TestPassed => "test.passed",
            Self::AgentSpawned => "agent.spawned",
            Self::AgentCompleted => "agent.completed",
            Self::ApprovalRequested => "approval.requested",
            Self::ApprovalGranted => "approval.granted",
            Self::ApprovalDenied => "approval.denied",
            Self::TaskCompleted => "task.completed",
            Self::TaskFailed => "task.failed",
            Self::TaskCancelled => "task.cancelled",
            Self::PhaseChanged => "phase.changed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "task.created" => Self::TaskCreated,
            "task.started" => Self::TaskStarted,
            "plan.created" => Self::PlanCreated,
            "context.retrieved" => Self::ContextRetrieved,
            "llm.request" => Self::LlmRequest,
            "llm.response" => Self::LlmResponse,
            "tool.started" => Self::ToolStarted,
            "tool.completed" => Self::ToolCompleted,
            "file.modified" => Self::FileModified,
            "test.started" => Self::TestStarted,
            "test.failed" => Self::TestFailed,
            "test.passed" => Self::TestPassed,
            "agent.spawned" => Self::AgentSpawned,
            "agent.completed" => Self::AgentCompleted,
            "approval.requested" => Self::ApprovalRequested,
            "approval.granted" => Self::ApprovalGranted,
            "approval.denied" => Self::ApprovalDenied,
            "task.completed" => Self::TaskCompleted,
            "task.failed" => Self::TaskFailed,
            "task.cancelled" => Self::TaskCancelled,
            "phase.changed" => Self::PhaseChanged,
            _ => return None,
        })
    }
}

/// Structured, queryable runtime event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: EventId,
    pub task_id: TaskId,
    pub kind: EventKind,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

impl Event {
    pub fn new(task_id: TaskId, kind: EventKind, payload: Value) -> Self {
        Self {
            id: EventId::new(),
            task_id,
            kind,
            payload,
            created_at: Utc::now(),
        }
    }
}
