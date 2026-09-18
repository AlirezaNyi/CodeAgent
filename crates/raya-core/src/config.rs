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
    /// Phase/role → named lane mapping (empty string = active lane).
    pub models: ModelsConfig,
    pub memory: MemoryConfig,
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
        if self.subagent.max_iterations == 0 {
            return Err(
                ConfigError::Validation("subagent.max_iterations must be >= 1".into()).into(),
            );
        }
        if self.subagent.max_tool_calls == 0 {
            return Err(
                ConfigError::Validation("subagent.max_tool_calls must be >= 1".into()).into(),
            );
        }
        if self.subagent.timeout_seconds == 0 {
            return Err(
                ConfigError::Validation("subagent.timeout_seconds must be >= 1".into()).into(),
            );
        }
        if self.subagent.max_review_rounds == 0 {
            return Err(
                ConfigError::Validation("subagent.max_review_rounds must be >= 1".into()).into(),
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
        if provider != "mock" && provider != "openai" && provider != "local" {
            return Err(ConfigError::Validation(format!(
                "llm.provider must be \"mock\", \"openai\", or \"local\", got \"{}\"",
                self.llm.provider
            ))
            .into());
        }
        self.llm.validate_lanes()?;
        self.models.validate_against_lanes(&self.llm)?;
        if self.memory.max_items == 0 {
            return Err(ConfigError::Validation("memory.max_items must be >= 1".into()).into());
        }
        if self.memory.max_tokens == 0 {
            return Err(ConfigError::Validation("memory.max_tokens must be >= 1".into()).into());
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
        if let Ok(v) = std::env::var("RAYA_LLM_LANE") {
            self.llm.lane = v;
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
    pub max_iterations: u32,
    pub max_tool_calls: u32,
    pub timeout_seconds: u64,
    pub max_review_rounds: u32,
    /// When true, spawn a Reviewer subagent after successful verification.
    pub review_on_finish: bool,
    /// When true, spawn a Debugger subagent after failed verification.
    pub debug_on_verify_fail: bool,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            max_parallel: 3,
            max_iterations: 6,
            max_tool_calls: 15,
            timeout_seconds: 300,
            max_review_rounds: 1,
            review_on_finish: false,
            debug_on_verify_fail: false,
        }
    }
}

/// Maps task phases / subagent roles to named LLM lanes.
///
/// Empty string means "use the active lane" (`llm.lane` / legacy top-level).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ModelsConfig {
    pub planning: String,
    pub coding: String,
    pub review: String,
    pub debug: String,
    pub summary: String,
}

impl ModelsConfig {
    /// All role → lane-name pairs (for iteration / CLI display).
    pub fn role_lanes(&self) -> [(&'static str, &str); 5] {
        [
            ("planning", &self.planning),
            ("coding", &self.coding),
            ("review", &self.review),
            ("debug", &self.debug),
            ("summary", &self.summary),
        ]
    }

    fn validate_against_lanes(&self, llm: &LlmConfig) -> Result<()> {
        for (role, lane_name) in self.role_lanes() {
            if lane_name.is_empty() {
                continue;
            }
            if llm.lanes.is_empty() {
                return Err(ConfigError::Validation(format!(
                    "models.{role} = \"{lane_name}\" but llm.lanes is empty; \
                     use empty string for the active/legacy lane"
                ))
                .into());
            }
            if !llm.lanes.iter().any(|l| l.name == *lane_name) {
                return Err(ConfigError::Validation(format!(
                    "models.{role} lane \"{lane_name}\" not found in llm.lanes"
                ))
                .into());
            }
        }
        Ok(())
    }

    /// Resolve the lane name for a role (empty → active lane name).
    pub fn lane_for_role(&self, role: &str, active_lane: &str) -> String {
        let named = match role {
            "planning" => self.planning.as_str(),
            "coding" => self.coding.as_str(),
            "review" => self.review.as_str(),
            "debug" => self.debug.as_str(),
            "summary" => self.summary.as_str(),
            _ => "",
        };
        if named.is_empty() {
            active_lane.to_string()
        } else {
            named.to_string()
        }
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

/// Selective memory retrieval bounds (RFC §15).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MemoryConfig {
    pub enabled: bool,
    pub max_items: u32,
    pub max_tokens: u64,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_items: 5,
            max_tokens: 2000,
        }
    }
}

/// Named OpenAI-compatible backend (local or remote).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmLane {
    pub name: String,
    pub base_url: String,
    pub model: String,
    /// Optional env var name for the API key (defaults to `RAYA_LLM_API_KEY` / `OPENAI_API_KEY`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
}

/// Resolved active lane after config / env selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLlmLane {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LlmConfig {
    /// `"mock"`, `"openai"`, or `"local"` (`local` uses the OpenAI-compatible client).
    pub provider: String,
    /// Active lane name when [`Self::lanes`] is non-empty.
    pub lane: String,
    pub base_url: String,
    pub model: String,
    pub timeout_seconds: u64,
    /// Named backends; empty means use top-level `base_url` / `model`.
    pub lanes: Vec<LlmLane>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "mock".into(),
            lane: String::new(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-4o-mini".into(),
            timeout_seconds: 120,
            lanes: Vec::new(),
        }
    }
}

impl LlmConfig {
    fn validate_lanes(&self) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for lane in &self.lanes {
            if lane.name.trim().is_empty() {
                return Err(
                    ConfigError::Validation("llm.lanes[].name must be non-empty".into()).into(),
                );
            }
            if lane.base_url.trim().is_empty() {
                return Err(ConfigError::Validation(format!(
                    "llm.lanes[{}].base_url must be non-empty",
                    lane.name
                ))
                .into());
            }
            if lane.model.trim().is_empty() {
                return Err(ConfigError::Validation(format!(
                    "llm.lanes[{}].model must be non-empty",
                    lane.name
                ))
                .into());
            }
            if !seen.insert(lane.name.clone()) {
                return Err(ConfigError::Validation(format!(
                    "duplicate llm.lanes name \"{}\"",
                    lane.name
                ))
                .into());
            }
        }
        if !self.lane.is_empty() && !self.lanes.is_empty() {
            let exists = self.lanes.iter().any(|l| l.name == self.lane);
            if !exists {
                return Err(ConfigError::Validation(format!(
                    "llm.lane \"{}\" not found in llm.lanes",
                    self.lane
                ))
                .into());
            }
        }
        Ok(())
    }

    /// Resolve the active lane (named entry or legacy top-level `base_url` / `model`).
    pub fn resolve_lane(&self) -> Result<ResolvedLlmLane> {
        if self.lanes.is_empty() {
            return Ok(ResolvedLlmLane {
                name: if self.lane.is_empty() {
                    "default".into()
                } else {
                    self.lane.clone()
                },
                base_url: self.base_url.clone(),
                model: self.model.clone(),
                api_key_env: None,
            });
        }
        let name = if self.lane.is_empty() {
            self.lanes[0].name.clone()
        } else {
            self.lane.clone()
        };
        let lane = self.lanes.iter().find(|l| l.name == name).ok_or_else(|| {
            ConfigError::Validation(format!("llm.lane \"{name}\" not found in llm.lanes"))
        })?;
        Ok(ResolvedLlmLane {
            name: lane.name.clone(),
            base_url: lane.base_url.clone(),
            model: lane.model.clone(),
            api_key_env: lane.api_key_env.clone(),
        })
    }

    /// Copy the active lane's `base_url` / `model` onto the top-level fields for consumers.
    pub fn apply_active_lane(&mut self) -> Result<()> {
        let resolved = self.resolve_lane()?;
        self.lane = resolved.name;
        self.base_url = resolved.base_url;
        self.model = resolved.model;
        Ok(())
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
    use std::path::Path;
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

    #[test]
    fn accepts_local_provider() {
        let mut c = Config::default();
        c.llm.provider = "local".into();
        c.validate().expect("local ok");
    }

    #[test]
    fn rejects_unknown_provider() {
        let mut c = Config::default();
        c.llm.provider = "anthropic".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn parses_lanes_and_resolves() {
        let text = r#"
[llm]
provider = "local"
lane = "ollama"
timeout_seconds = 60

[[llm.lanes]]
name = "ollama"
base_url = "http://127.0.0.1:11434/v1"
model = "llama3.2"

[[llm.lanes]]
name = "lmstudio"
base_url = "http://127.0.0.1:1234/v1"
model = "local-model"
"#;
        let mut cfg = Config::from_toml(text, Path::new("test.toml")).unwrap();
        cfg.validate().unwrap();
        let resolved = cfg.llm.resolve_lane().unwrap();
        assert_eq!(resolved.name, "ollama");
        assert_eq!(resolved.base_url, "http://127.0.0.1:11434/v1");
        assert_eq!(resolved.model, "llama3.2");
        cfg.llm.apply_active_lane().unwrap();
        assert_eq!(cfg.llm.model, "llama3.2");
    }

    #[test]
    fn rejects_unknown_lane_name() {
        let mut c = Config::default();
        c.llm.provider = "openai".into();
        c.llm.lane = "missing".into();
        c.llm.lanes = vec![LlmLane {
            name: "ollama".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            model: "llama3.2".into(),
            api_key_env: None,
        }];
        assert!(c.validate().is_err());
    }

    #[test]
    fn legacy_base_url_when_lanes_empty() {
        let c = Config::default();
        let resolved = c.llm.resolve_lane().unwrap();
        assert_eq!(resolved.name, "default");
        assert_eq!(resolved.base_url, c.llm.base_url);
    }

    #[test]
    fn rejects_zero_subagent_iterations() {
        let mut c = Config::default();
        c.subagent.max_iterations = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_models_lane_when_lanes_empty() {
        let mut c = Config::default();
        c.models.coding = "ollama".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_unknown_models_lane() {
        let mut c = Config::default();
        c.llm.provider = "local".into();
        c.llm.lanes = vec![LlmLane {
            name: "ollama".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            model: "llama3.2".into(),
            api_key_env: None,
        }];
        c.models.review = "missing".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn accepts_models_lane_when_present() {
        let mut c = Config::default();
        c.llm.provider = "local".into();
        c.llm.lanes = vec![
            LlmLane {
                name: "ollama".into(),
                base_url: "http://127.0.0.1:11434/v1".into(),
                model: "llama3.2".into(),
                api_key_env: None,
            },
            LlmLane {
                name: "strong".into(),
                base_url: "http://127.0.0.1:1234/v1".into(),
                model: "coder".into(),
                api_key_env: None,
            },
        ];
        c.models.coding = "strong".into();
        c.models.planning = "ollama".into();
        c.validate().expect("models lanes ok");
        assert_eq!(c.models.lane_for_role("coding", "ollama"), "strong");
        assert_eq!(c.models.lane_for_role("review", "ollama"), "ollama");
    }

    #[test]
    fn parses_subagent_extended_fields() {
        let text = r#"
[subagent]
max_parallel = 2
max_iterations = 4
max_tool_calls = 10
timeout_seconds = 120
max_review_rounds = 2
review_on_finish = true
debug_on_verify_fail = true

[models]
coding = "strong"
review = "cheap"

[llm]
provider = "local"
lane = "cheap"

[[llm.lanes]]
name = "cheap"
base_url = "http://127.0.0.1:11434/v1"
model = "small"

[[llm.lanes]]
name = "strong"
base_url = "http://127.0.0.1:1234/v1"
model = "large"
"#;
        let cfg = Config::from_toml(text, Path::new("test.toml")).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.subagent.max_parallel, 2);
        assert_eq!(cfg.subagent.max_iterations, 4);
        assert!(cfg.subagent.review_on_finish);
        assert!(cfg.subagent.debug_on_verify_fail);
        assert_eq!(cfg.models.coding, "strong");
        assert_eq!(cfg.models.review, "cheap");
    }
}
