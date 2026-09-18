//! Structured agent decisions validated from LLM output.

use serde::{Deserialize, Serialize};

use super::plan::ExecutionPlan;
use super::tool::ToolCall;

/// Validated structured output from the agent LLM (REQ-PLAN-004).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentDecision {
    /// Produce or update an execution plan.
    Plan { plan: ExecutionPlan },
    /// Invoke a single tool.
    ToolCall { call: ToolCall },
    /// Task is finished successfully.
    Finish { summary: String },
    /// Verification failed; request another fix iteration.
    NeedsFix { reason: String },
}

impl AgentDecision {
    /// Parse and validate a decision from JSON text.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
        Self::from_value(value)
    }

    pub fn from_value(value: serde_json::Value) -> Result<Self, String> {
        serde_json::from_value(value).map_err(|e| format!("invalid agent decision: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::plan::{PlanStep, VerificationStrategy};
    use serde_json::json;

    #[test]
    fn parses_finish() {
        let d = AgentDecision::from_json(r#"{"type":"finish","summary":"done"}"#).unwrap();
        assert_eq!(
            d,
            AgentDecision::Finish {
                summary: "done".into()
            }
        );
    }

    #[test]
    fn rejects_malformed() {
        assert!(AgentDecision::from_json("not json").is_err());
        assert!(AgentDecision::from_json(r#"{"type":"explode"}"#).is_err());
    }

    #[test]
    fn parses_plan() {
        let plan = ExecutionPlan::new(
            vec![PlanStep {
                id: "1".into(),
                description: "write".into(),
                expected_tools: vec!["filesystem.write".into()],
            }],
            VerificationStrategy::Test,
        );
        let v = json!({"type":"plan","plan": plan});
        let d = AgentDecision::from_value(v).unwrap();
        assert!(matches!(d, AgentDecision::Plan { .. }));
    }
}
