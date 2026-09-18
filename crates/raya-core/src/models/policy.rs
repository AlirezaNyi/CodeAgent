//! Policy decision and risk classification models.

use serde::{Deserialize, Serialize};

/// Risk class for a tool operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskClass {
    Read,
    LowWrite,
    BuildTest,
    GitCommit,
    Network,
    Destructive,
    Production,
    CredentialAccess,
}

impl RiskClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "READ",
            Self::LowWrite => "LOW_WRITE",
            Self::BuildTest => "BUILD_TEST",
            Self::GitCommit => "GIT_COMMIT",
            Self::Network => "NETWORK",
            Self::Destructive => "DESTRUCTIVE",
            Self::Production => "PRODUCTION",
            Self::CredentialAccess => "CREDENTIAL_ACCESS",
        }
    }
}

/// Outcome of a policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PolicyDecision {
    Allow,
    Deny { reason: String },
    RequireApproval { reason: String },
}

impl PolicyDecision {
    pub fn is_allow(&self) -> bool {
        matches!(self, Self::Allow)
    }

    pub fn is_deny(&self) -> bool {
        matches!(self, Self::Deny { .. })
    }

    pub fn requires_approval(&self) -> bool {
        matches!(self, Self::RequireApproval { .. })
    }
}
