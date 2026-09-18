//! Project configuration loaded from `.raya/config.toml`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, Result};

/// Full RAYA project configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub project: ProjectConfig,
    pub agent: AgentConfig,
    pub context: ContextConfig,
    pub subagent: SubagentConfig,
    pub policy: PolicyConfig,
    pub git: GitConfig,
    pub resources: ResourcesConfig,
    pub llm: LlmConfig,
    pub server: ServerConfig,
}

impl Config {
    /// Load configuration from a project root.
    ///
    /// Looks for `.raya/config.toml`. Missing file yields defaults.
    /// Environment variables `RAYA_*` override selected fields after load.
    pub fn load(project_root: &Path) -> Result<Self> {
        let path = project_root.join(".raya").join("config.toml");
        let mut config = if path.exists() {
            Self::from_file(&path)?
        } else {
            Self::default()
        };
        config.apply_env_overrides();
        config.validate()?;
        Ok(config)
    }

    /// Parse configuration from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_toml(&text, path)
    }

    /// Parse configuration from a TOML string.
    pub fn from_toml(text: &str, path: &Path) -> Result<Self> {
        let config: Config = toml::from_str(text).map_err(|source| ConfigError::Parse {
            path: path.display().to_string(),
            source,
        })?;
        Ok(config)
    }

    /// Validate resource and policy bounds.
    pub fn validate(&self) -> Result<()> {
        if self.agent.max_iterations == 0 {
            return Err(ConfigError::Validation("agent.max_iterations must be >= 1".into()).into());
        }
        if self.agent.max_tool_calls == 0 {
            return Err(ConfigError::Validation("agent.max_tool_calls must be >= 1".into()).into());
        }
        if self.agent.timeout_seconds == 0 {
            return Err(
                ConfigError::Validation("agent.timeout_seconds must be >= 1".into()).into(),
            );
        }
        if self.context.max_tokens == 0 {
            return Err(ConfigError::Validation("context.max_tokens must be >= 1".into()).into());
        }
        if self.context.max_files == 0 {
            return Err(ConfigError::Validation("context.max_files must be >= 1".into()).into());
        }
        if self.subagent.max_parallel == 0 {
            return Err(
                ConfigError::Validation("subagent.max_parallel must be >= 1".into()).into(),
            );
        }
        if self.resources.max_parallel_tools == 0 {
            return Err(ConfigError::Validation(
                "resources.max_parallel_tools must be >= 1".into(),
            )
            .into());
        }
        if self.resources.max_processes == 0 {
            return Err(
                ConfigError::Validation("resources.max_processes must be >= 1".into()).into(),
            );
        }
        if self.server.port == 0 {
            return Err(ConfigError::Validation("server.port must be >= 1".into()).into());
        }
        if !self.server.bind_loopback && !self.server.allow_remote {
            return Err(ConfigError::Validation(
                "server.allow_remote must be true when bind_loopback is false".into(),
            )
            .into());
        }
        let provider = self.llm.provider.to_ascii_lowercase();
        if provider != "mock" && provider != "openai" {
            return Err(ConfigError::Validation(format!(
                "llm.provider must be \"mock\" or \"openai\", got \"{}\"",
                self.llm.provider
            ))
            .into());
        }
        Ok(())
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("RAYA_LLM_PROVIDER") {
            self.llm.provider = v;
        }
        if let Ok(v) = std::env::var("RAYA_LLM_BASE_URL") {
            self.llm.base_url = v;
        }
        if let Ok(v) = std::env::var("RAYA_LLM_MODEL") {
            self.llm.model = v;
        }
        if let Ok(v) = std::env::var("RAYA_SERVER_PORT")
            && let Ok(port) = v.parse::<u16>()
        {
            self.server.port = port;
        }
        if let Ok(v) = std::env::var("RAYA_LOG_FORMAT") {
            self.server.log_format = v;
        }
    }

    /// Path to the SQLite database under the project root.
    pub fn database_path(&self, project_root: &Path) -> PathBuf {
        project_root.join(".raya").join("raya.db")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProjectConfig {
    pub language: String,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            language: "auto".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AgentConfig {
    pub max_iterations: u32,
    pub max_tool_calls: u32,
    pub timeout_seconds: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_iterations: 12,
            max_tool_calls: 40,
            timeout_seconds: 1800,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ContextConfig {
    pub max_tokens: u64,
    pub max_files: u32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_tokens: 40_000,
            max_files: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SubagentConfig {
    pub max_parallel: u32,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self { max_parallel: 3 }
    }
}

/// Policy action for a risk class.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PolicyAction {
    #[default]
    Auto,
    Approval,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PolicyConfig {
    pub shell: PolicyAction,
    pub git_commit: PolicyAction,
    pub network: PolicyAction,
    pub destructive: PolicyAction,
    pub credentials: PolicyAction,
    pub write: PolicyAction,
    pub read: PolicyAction,
    pub build_test: PolicyAction,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            shell: PolicyAction::Approval,
            git_commit: PolicyAction::Approval,
            network: PolicyAction::Approval,
            destructive: PolicyAction::Deny,
            credentials: PolicyAction::Deny,
            write: PolicyAction::Auto,
            read: PolicyAction::Auto,
            build_test: PolicyAction::Auto,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GitConfig {
    pub auto_commit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ResourcesConfig {
    pub max_memory_mb: u64,
    pub max_parallel_tools: u32,
    pub max_parallel_agents: u32,
    pub max_processes: u32,
}

impl Default for ResourcesConfig {
    fn default() -> Self {
        Self {
            max_memory_mb: 700,
            max_parallel_tools: 4,
            max_parallel_agents: 3,
            max_processes: 8,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LlmConfig {
    /// `"mock"` or `"openai"`.
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub timeout_seconds: u64,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "mock".into(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-4o-mini".into(),
            timeout_seconds: 120,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub bind_loopback: bool,
    pub allow_remote: bool,
    /// `"pretty"` or `"json"`.
    pub log_format: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 7319,
            bind_loopback: true,
            allow_remote: false,
            log_format: "pretty".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn defaults_validate() {
        Config::default().validate().expect("defaults ok");
    }

    #[test]
    fn rejects_zero_iterations() {
        let mut c = Config::default();
        c.agent.max_iterations = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn loads_from_file() {
        let dir = tempdir().unwrap();
        let raya = dir.path().join(".raya");
        std::fs::create_dir_all(&raya).unwrap();
        let path = raya.join("config.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[agent]
max_iterations = 5

[llm]
provider = "mock"
"#
        )
        .unwrap();
        let cfg = Config::load(dir.path()).unwrap();
        assert_eq!(cfg.agent.max_iterations, 5);
        assert_eq!(cfg.llm.provider, "mock");
    }

    #[test]
    fn missing_file_uses_defaults() {
        let dir = tempdir().unwrap();
        let cfg = Config::load(dir.path()).unwrap();
        assert_eq!(cfg.agent.max_iterations, 12);
    }
}
