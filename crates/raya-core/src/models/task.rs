//! Task lifecycle models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ids::{ProjectId, TaskId};
use super::plan::ExecutionPlan;
use super::tool::ToolCall;
use crate::models::llm::Message;

/// High-level task status derived from the current phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Cancelled,
}

/// Agent task phase in the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPhase {
    Created,
    Planning,
    ContextBuilding,
    Executing,
    Verifying,
    Fixing,
    WaitingApproval,
    Completed,
    Failed,
    Cancelled,
}

impl TaskPhase {
    /// Whether a transition from `self` to `next` is allowed.
    pub fn can_transition_to(self, next: TaskPhase) -> bool {
        use TaskPhase::*;
        matches!(
            (self, next),
            (Created, Planning)
                | (Created, Cancelled)
                | (Planning, ContextBuilding)
                | (Planning, Failed)
                | (Planning, Cancelled)
                | (Planning, WaitingApproval)
                | (ContextBuilding, Executing)
                | (ContextBuilding, Failed)
                | (ContextBuilding, Cancelled)
                | (ContextBuilding, WaitingApproval)
                | (Executing, Verifying)
                | (Executing, WaitingApproval)
                | (Executing, Failed)
                | (Executing, Cancelled)
                | (Executing, Fixing)
                | (Verifying, Completed)
                | (Verifying, Fixing)
                | (Verifying, Failed)
                | (Verifying, Cancelled)
                | (Fixing, Executing)
                | (Fixing, Failed)
                | (Fixing, Cancelled)
                | (WaitingApproval, Executing)
                | (WaitingApproval, Cancelled)
                | (WaitingApproval, Failed) // Any non-terminal may fail or cancel — covered above.
                                            // Terminal states have no outgoing transitions.
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskPhase::Completed | TaskPhase::Failed | TaskPhase::Cancelled
        )
    }

    pub fn status(self) -> TaskStatus {
        match self {
            TaskPhase::Created => TaskStatus::Pending,
            TaskPhase::WaitingApproval => TaskStatus::WaitingApproval,
            TaskPhase::Completed => TaskStatus::Completed,
            TaskPhase::Failed => TaskStatus::Failed,
            TaskPhase::Cancelled => TaskStatus::Cancelled,
            TaskPhase::Planning
            | TaskPhase::ContextBuilding
            | TaskPhase::Executing
            | TaskPhase::Verifying
            | TaskPhase::Fixing => TaskStatus::Running,
        }
    }
}

/// Persistent agent task record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentTask {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub request: String,
    pub phase: TaskPhase,
    pub plan: Option<ExecutionPlan>,
    pub current_step: Option<String>,
    pub max_iterations: u32,
    pub max_tool_calls: u32,
    pub max_tokens: u64,
    pub context_token_budget: u64,
    pub deadline: Option<DateTime<Utc>>,
    pub iterations: u32,
    pub tool_calls: u32,
    pub tokens_used: u64,
    pub error: Option<String>,
    pub cancel_requested: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AgentTask {
    pub fn new(
        project_id: ProjectId,
        request: impl Into<String>,
        max_iterations: u32,
        max_tool_calls: u32,
        max_tokens: u64,
        context_token_budget: u64,
        deadline: Option<DateTime<Utc>>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: TaskId::new(),
            project_id,
            request: request.into(),
            phase: TaskPhase::Created,
            plan: None,
            current_step: None,
            max_iterations,
            max_tool_calls,
            max_tokens,
            context_token_budget,
            deadline,
            iterations: 0,
            tool_calls: 0,
            tokens_used: 0,
            error: None,
            cancel_requested: false,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn status(&self) -> TaskStatus {
        self.phase.status()
    }
}

/// Durable orchestrator checkpoint for approval resume (Phase 3 Slice B).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskCheckpoint {
    pub task_id: TaskId,
    pub pending_call: Option<ToolCall>,
    pub messages: Vec<Message>,
    pub review_rounds: u32,
}

impl TaskCheckpoint {
    pub fn new(task_id: TaskId, messages: Vec<Message>, pending_call: Option<ToolCall>) -> Self {
        Self {
            task_id,
            pending_call,
            messages,
            review_rounds: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_transitions() {
        assert!(TaskPhase::Created.can_transition_to(TaskPhase::Planning));
        assert!(TaskPhase::Planning.can_transition_to(TaskPhase::ContextBuilding));
        assert!(TaskPhase::ContextBuilding.can_transition_to(TaskPhase::Executing));
        assert!(TaskPhase::Executing.can_transition_to(TaskPhase::Verifying));
        assert!(TaskPhase::Verifying.can_transition_to(TaskPhase::Completed));
        assert!(TaskPhase::Verifying.can_transition_to(TaskPhase::Fixing));
        assert!(TaskPhase::Fixing.can_transition_to(TaskPhase::Executing));
    }

    #[test]
    fn terminal_has_no_exit() {
        assert!(!TaskPhase::Completed.can_transition_to(TaskPhase::Executing));
        assert!(!TaskPhase::Failed.can_transition_to(TaskPhase::Planning));
        assert!(!TaskPhase::Cancelled.can_transition_to(TaskPhase::Created));
    }

    #[test]
    fn invalid_skip_disallowed() {
        assert!(!TaskPhase::Created.can_transition_to(TaskPhase::Executing));
        assert!(!TaskPhase::Planning.can_transition_to(TaskPhase::Completed));
    }

    #[test]
    fn status_mapping() {
        assert_eq!(TaskPhase::Created.status(), TaskStatus::Pending);
        assert_eq!(TaskPhase::Executing.status(), TaskStatus::Running);
        assert_eq!(
            TaskPhase::WaitingApproval.status(),
            TaskStatus::WaitingApproval
        );
        assert_eq!(TaskPhase::Completed.status(), TaskStatus::Completed);
    }
}
