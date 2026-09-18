//! Subagent roles, tool allowlists, and model-role mapping.

use raya_llm::ModelRole;
use serde::{Deserialize, Serialize};

/// Ephemeral worker role (RFC §12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentRole {
    Planner,
    Coder,
    Reviewer,
    Debugger,
}

impl SubagentRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planner => "planner",
            Self::Coder => "coder",
            Self::Reviewer => "reviewer",
            Self::Debugger => "debugger",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "planner" => Self::Planner,
            "coder" => Self::Coder,
            "reviewer" => Self::Reviewer,
            "debugger" => Self::Debugger,
            _ => return None,
        })
    }

    /// Tools permitted for this role (still gated by PolicyEngine).
    pub fn allowed_tools(self) -> &'static [&'static str] {
        match self {
            Self::Planner => &["filesystem.read", "search.grep", "git.status", "git.diff"],
            Self::Coder => &[
                "filesystem.read",
                "filesystem.write",
                "filesystem.patch",
                "search.grep",
                "git.status",
                "git.diff",
                "test.run",
                "build.run",
            ],
            Self::Reviewer => &["filesystem.read", "search.grep", "git.status", "git.diff"],
            Self::Debugger => &[
                "filesystem.read",
                "search.grep",
                "git.diff",
                "test.run",
                "build.run",
            ],
        }
    }

    pub fn permits_tool(self, name: &str) -> bool {
        self.allowed_tools().contains(&name)
    }

    pub fn model_role(self) -> ModelRole {
        match self {
            Self::Planner => ModelRole::Planning,
            Self::Coder => ModelRole::Coding,
            Self::Reviewer => ModelRole::Review,
            Self::Debugger => ModelRole::Debug,
        }
    }

    pub fn prompt(self) -> &'static str {
        match self {
            Self::Planner => include_str!("../../../../prompts/subagent/planner.md"),
            Self::Coder => include_str!("../../../../prompts/subagent/coder.md"),
            Self::Reviewer => include_str!("../../../../prompts/subagent/reviewer.md"),
            Self::Debugger => include_str!("../../../../prompts/subagent/debugger.md"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coder_allows_write_not_shell() {
        assert!(SubagentRole::Coder.permits_tool("filesystem.write"));
        assert!(!SubagentRole::Coder.permits_tool("shell.exec"));
    }

    #[test]
    fn reviewer_is_read_only() {
        assert!(SubagentRole::Reviewer.permits_tool("filesystem.read"));
        assert!(!SubagentRole::Reviewer.permits_tool("filesystem.write"));
        assert!(!SubagentRole::Reviewer.permits_tool("test.run"));
    }

    #[test]
    fn parse_roles() {
        assert_eq!(SubagentRole::parse("Planner"), Some(SubagentRole::Planner));
        assert!(SubagentRole::parse("unknown").is_none());
    }
}
