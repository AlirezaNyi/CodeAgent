//! Policy engine: classify tool risk and decide allow / deny / approval.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use raya_core::{PolicyAction, PolicyConfig, PolicyDecision, RiskClass};
use serde_json::Value;
use tracing::debug;

/// Classify a tool invocation into a risk class.
pub fn classify(tool_name: &str, input: &Value) -> RiskClass {
    let text = input.to_string().to_ascii_lowercase();

    // Credential paths
    if looks_like_credential_path(tool_name, input, &text) {
        return RiskClass::CredentialAccess;
    }

    // Destructive patterns in shell / git
    if is_destructive(tool_name, &text) {
        return RiskClass::Destructive;
    }

    // Network
    if is_network(tool_name, &text) {
        return RiskClass::Network;
    }

    // Production-ish
    if text.contains("kubectl") || text.contains("production") || text.contains("--prod") {
        return RiskClass::Production;
    }

    match tool_name {
        "filesystem.read" | "search.grep" | "git.status" | "git.diff" | "git.log"
        | "context.search" => RiskClass::Read,
        "filesystem.write" | "filesystem.patch" => RiskClass::LowWrite,
        "test.run" | "build.run" => RiskClass::BuildTest,
        "git.commit" => RiskClass::GitCommit,
        "shell.exec" => {
            // Generic shell is treated as needing shell policy; classify as network/destructive
            // already handled. Default shell → treat as LowWrite-equivalent via shell policy.
            // Use a dedicated path: shell maps via shell policy action, risk = LowWrite unless
            // elevated above.
            RiskClass::LowWrite
        }
        _ => RiskClass::LowWrite,
    }
}

fn looks_like_credential_path(tool_name: &str, input: &Value, text: &str) -> bool {
    if !matches!(
        tool_name,
        "filesystem.read" | "filesystem.write" | "filesystem.patch" | "shell.exec"
    ) {
        return false;
    }
    let path = input
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let hay = format!("{path} {text}");
    hay.contains(".env")
        || hay.contains(".pem")
        || hay.contains("id_rsa")
        || hay.contains("id_ed25519")
        || hay.contains("credentials")
        || hay.contains("secret")
        || hay.contains(".aws/credentials")
}

fn is_destructive(tool_name: &str, text: &str) -> bool {
    if tool_name == "git.commit" {
        return false;
    }
    let patterns = [
        "rm -rf",
        "rm -fr",
        "git push --force",
        "git push -f",
        "git reset --hard",
        "mkfs",
        "dd if=",
        ":(){",
        "shutdown",
        "reboot",
        "diskutil erase",
    ];
    patterns.iter().any(|p| text.contains(p))
}

fn is_network(tool_name: &str, text: &str) -> bool {
    if tool_name.starts_with("network.") {
        return true;
    }
    let patterns = [
        "curl ", "wget ", "nc ", "ssh ", "scp ", "http://", "https://",
    ];
    tool_name == "shell.exec" && patterns.iter().any(|p| text.contains(p))
}

/// Policy engine driven by configuration.
#[derive(Debug, Clone)]
pub struct PolicyEngine {
    config: PolicyConfig,
}

impl PolicyEngine {
    pub fn new(config: PolicyConfig) -> Self {
        Self { config }
    }

    pub fn from_defaults() -> Self {
        Self::new(PolicyConfig::default())
    }

    /// Evaluate a tool call. Denied operations must never execute.
    pub fn evaluate(&self, tool_name: &str, input: &Value) -> PolicyDecision {
        let risk = classify(tool_name, input);
        let action = self.action_for(tool_name, risk);
        let decision = match action {
            PolicyAction::Auto => PolicyDecision::Allow,
            PolicyAction::Approval => PolicyDecision::RequireApproval {
                reason: format!(
                    "tool `{tool_name}` classified as {} requires approval",
                    risk.as_str()
                ),
            },
            PolicyAction::Deny => PolicyDecision::Deny {
                reason: format!(
                    "tool `{tool_name}` classified as {} is denied by policy",
                    risk.as_str()
                ),
            },
        };
        debug!(
            tool = tool_name,
            risk = risk.as_str(),
            ?decision,
            "policy evaluated"
        );
        decision
    }

    fn action_for(&self, tool_name: &str, risk: RiskClass) -> PolicyAction {
        // Shell tool always uses shell policy unless elevated to deny/destructive.
        if tool_name == "shell.exec" {
            return match risk {
                RiskClass::Destructive | RiskClass::Production | RiskClass::CredentialAccess => {
                    self.action_for_risk(risk)
                }
                RiskClass::Network => self.config.network,
                _ => self.config.shell,
            };
        }

        self.action_for_risk(risk)
    }

    fn action_for_risk(&self, risk: RiskClass) -> PolicyAction {
        match risk {
            RiskClass::Read => self.config.read,
            RiskClass::LowWrite => self.config.write,
            RiskClass::BuildTest => self.config.build_test,
            RiskClass::GitCommit => self.config.git_commit,
            RiskClass::Network => self.config.network,
            RiskClass::Destructive => self.config.destructive,
            RiskClass::Production => self.config.destructive,
            RiskClass::CredentialAccess => self.config.credentials,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_is_auto() {
        let engine = PolicyEngine::from_defaults();
        let d = engine.evaluate("filesystem.read", &json!({"path": "src/main.rs"}));
        assert!(d.is_allow());
    }

    #[test]
    fn shell_requires_approval_by_default() {
        let engine = PolicyEngine::from_defaults();
        let d = engine.evaluate("shell.exec", &json!({"command": "ls"}));
        assert!(d.requires_approval());
    }

    #[test]
    fn destructive_denied() {
        let engine = PolicyEngine::from_defaults();
        let d = engine.evaluate("shell.exec", &json!({"command": "rm -rf /"}));
        assert!(d.is_deny());
        assert_eq!(
            classify("shell.exec", &json!({"command": "rm -rf /"})),
            RiskClass::Destructive
        );
    }

    #[test]
    fn credentials_denied() {
        let engine = PolicyEngine::from_defaults();
        let d = engine.evaluate("filesystem.read", &json!({"path": ".env"}));
        assert!(d.is_deny());
    }

    #[test]
    fn network_shell_approval() {
        let engine = PolicyEngine::from_defaults();
        let d = engine.evaluate(
            "shell.exec",
            &json!({"command": "curl https://example.com"}),
        );
        assert!(d.requires_approval());
    }

    #[test]
    fn git_commit_approval() {
        let engine = PolicyEngine::from_defaults();
        let d = engine.evaluate("git.commit", &json!({"message": "feat: x"}));
        assert!(d.requires_approval());
    }

    #[test]
    fn write_auto() {
        let engine = PolicyEngine::from_defaults();
        let d = engine.evaluate(
            "filesystem.write",
            &json!({"path": "src/a.rs", "content": "x"}),
        );
        assert!(d.is_allow());
    }
}
