//! Subagent brief and outcome types.

use serde::{Deserialize, Serialize};

use raya_core::ExecutionPlan;

use super::role::SubagentRole;

/// Minimal work order for a subagent (not the parent context dump).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentBrief {
    pub role: SubagentRole,
    pub objective: String,
    /// Relative paths to include as bounded context.
    pub context_paths: Vec<String>,
    /// Extra free-text context (e.g. verification output, parent summary).
    pub extra_context: String,
}

impl SubagentBrief {
    pub fn new(role: SubagentRole, objective: impl Into<String>) -> Self {
        Self {
            role,
            objective: objective.into(),
            context_paths: Vec::new(),
            extra_context: String::new(),
        }
    }

    pub fn with_paths(mut self, paths: Vec<String>) -> Self {
        self.context_paths = paths;
        self
    }

    pub fn with_extra(mut self, extra: impl Into<String>) -> Self {
        self.extra_context = extra.into();
        self
    }
}

/// Structured result of a subagent run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentVerdict {
    /// Reviewer approved / successful report.
    Approve,
    /// Needs another fix iteration.
    NeedsFix { reason: String },
    /// Planner produced a plan.
    Plan(ExecutionPlan),
    /// Generic completion report (coder/debugger).
    Report,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentOutcome {
    pub agent_id: String,
    pub role: SubagentRole,
    pub ok: bool,
    pub summary: String,
    pub verdict: SubagentVerdict,
    pub iterations: u32,
    pub tool_calls: u32,
    pub tokens_used: u64,
    pub duration_ms: u64,
}
