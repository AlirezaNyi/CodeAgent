//! Execution plan models.

use serde::{Deserialize, Serialize};

/// Structured execution plan produced by the planner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionPlan {
    pub steps: Vec<PlanStep>,
    pub verification: VerificationStrategy,
    pub summary: Option<String>,
}

impl ExecutionPlan {
    pub fn new(steps: Vec<PlanStep>, verification: VerificationStrategy) -> Self {
        Self {
            steps,
            verification,
            summary: None,
        }
    }
}

/// One ordered step in an execution plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanStep {
    pub id: String,
    pub description: String,
    pub expected_tools: Vec<String>,
}

/// How the agent should verify work after execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStrategy {
    None,
    #[default]
    Test,
    Build,
    TestAndBuild,
    Custom {
        command: String,
    },
}
