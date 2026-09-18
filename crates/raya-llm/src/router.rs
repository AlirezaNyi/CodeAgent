//! Role-based model router over named LLM lanes.

use std::collections::HashMap;
use std::sync::Arc;

use raya_core::{Config, ResolvedLlmLane};
use tokio_util::sync::CancellationToken;

use crate::mock::MockProvider;
use crate::openai::OpenAiCompatibleProvider;
use crate::provider::{LlmError, LlmProvider, LlmResult};

/// Task phase / subagent role that selects a model lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelRole {
    Planning,
    Coding,
    Review,
    Debug,
    Summary,
}

impl ModelRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Coding => "coding",
            Self::Review => "review",
            Self::Debug => "debug",
            Self::Summary => "summary",
        }
    }

    pub fn all() -> [Self; 5] {
        [
            Self::Planning,
            Self::Coding,
            Self::Review,
            Self::Debug,
            Self::Summary,
        ]
    }
}

/// Maps [`ModelRole`] → provider instance and model id.
pub struct ModelRouter {
    default: Arc<dyn LlmProvider>,
    by_role: HashMap<ModelRole, Arc<dyn LlmProvider>>,
    models: HashMap<ModelRole, String>,
    lanes: HashMap<ModelRole, String>,
}

impl ModelRouter {
    /// Wrap a single provider for every role (mock / tests / legacy callers).
    pub fn single(provider: Arc<dyn LlmProvider>) -> Self {
        let mut by_role = HashMap::new();
        let mut models = HashMap::new();
        let mut lanes = HashMap::new();
        for role in ModelRole::all() {
            by_role.insert(role, provider.clone());
            models.insert(role, String::new());
            lanes.insert(role, "default".into());
        }
        Self {
            default: provider,
            by_role,
            models,
            lanes,
        }
    }

    /// Build from project config.
    ///
    /// - `mock` → one shared [`MockProvider::default_script`] for all roles.
    /// - `openai` / `local` → one [`OpenAiCompatibleProvider`] per unique lane (deduped).
    pub fn from_config(config: &Config, cancel: CancellationToken) -> LlmResult<Self> {
        let provider = config.llm.provider.to_ascii_lowercase();
        match provider.as_str() {
            "mock" => Ok(Self::single(Arc::new(MockProvider::default_script()))),
            "openai" | "local" => Self::from_openai_config(config, cancel),
            other => Err(LlmError::Message(format!(
                "unsupported llm.provider: {other}"
            ))),
        }
    }

    fn from_openai_config(config: &Config, cancel: CancellationToken) -> LlmResult<Self> {
        let active = config
            .llm
            .resolve_lane()
            .map_err(|e| LlmError::Message(e.to_string()))?;

        // Collect unique lane names needed by roles.
        let mut needed: HashMap<String, ResolvedLlmLane> = HashMap::new();
        needed.insert(active.name.clone(), active.clone());

        for role in ModelRole::all() {
            let lane_name = config.models.lane_for_role(role.as_str(), &active.name);
            if needed.contains_key(&lane_name) {
                continue;
            }
            let resolved = resolve_named_lane(config, &lane_name)?;
            needed.insert(lane_name, resolved);
        }

        // Build one provider per unique lane.
        let mut providers: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();
        for (name, lane) in &needed {
            let p = OpenAiCompatibleProvider::from_resolved(
                lane,
                config.llm.timeout_seconds,
                cancel.clone(),
            )?;
            providers.insert(name.clone(), Arc::new(p));
        }

        let default = providers
            .get(&active.name)
            .cloned()
            .ok_or_else(|| LlmError::Message("active lane provider missing".into()))?;

        let mut by_role = HashMap::new();
        let mut models = HashMap::new();
        let mut lanes = HashMap::new();
        for role in ModelRole::all() {
            let lane_name = config.models.lane_for_role(role.as_str(), &active.name);
            let lane = needed
                .get(&lane_name)
                .ok_or_else(|| LlmError::Message(format!("lane \"{lane_name}\" not resolved")))?;
            let provider = providers.get(&lane_name).cloned().ok_or_else(|| {
                LlmError::Message(format!("provider for \"{lane_name}\" missing"))
            })?;
            by_role.insert(role, provider);
            models.insert(role, lane.model.clone());
            lanes.insert(role, lane_name);
        }

        Ok(Self {
            default,
            by_role,
            models,
            lanes,
        })
    }

    pub fn provider_for(&self, role: ModelRole) -> Arc<dyn LlmProvider> {
        self.by_role
            .get(&role)
            .cloned()
            .unwrap_or_else(|| self.default.clone())
    }

    /// Model id for the role (empty when using mock / single-provider wrap).
    pub fn model_for(&self, role: ModelRole) -> String {
        self.models.get(&role).cloned().unwrap_or_default()
    }

    /// Lane name selected for the role.
    pub fn lane_for(&self, role: ModelRole) -> String {
        self.lanes
            .get(&role)
            .cloned()
            .unwrap_or_else(|| "default".into())
    }

    /// Snapshot of role → (lane, model) for CLI display.
    pub fn routes(&self) -> Vec<(ModelRole, String, String)> {
        ModelRole::all()
            .into_iter()
            .map(|role| (role, self.lane_for(role), self.model_for(role)))
            .collect()
    }
}

fn resolve_named_lane(config: &Config, name: &str) -> LlmResult<ResolvedLlmLane> {
    if config.llm.lanes.is_empty() {
        return config
            .llm
            .resolve_lane()
            .map_err(|e| LlmError::Message(e.to_string()));
    }
    let lane = config
        .llm
        .lanes
        .iter()
        .find(|l| l.name == name)
        .ok_or_else(|| LlmError::Message(format!("llm.lane \"{name}\" not found in llm.lanes")))?;
    Ok(ResolvedLlmLane {
        name: lane.name.clone(),
        base_url: lane.base_url.clone(),
        model: lane.model.clone(),
        api_key_env: lane.api_key_env.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use raya_core::{LlmLane, ModelsConfig};

    #[test]
    fn single_routes_all_roles() {
        let router = ModelRouter::single(Arc::new(MockProvider::default_script()));
        for role in ModelRole::all() {
            let _ = router.provider_for(role);
            assert_eq!(router.lane_for(role), "default");
        }
    }

    #[tokio::test]
    async fn from_config_mock() {
        let config = Config::default();
        let router = ModelRouter::from_config(&config, CancellationToken::new()).unwrap();
        assert!(std::ptr::eq(
            Arc::as_ptr(&router.provider_for(ModelRole::Coding)) as *const (),
            Arc::as_ptr(&router.provider_for(ModelRole::Review)) as *const ()
        ));
    }

    #[test]
    fn from_config_lanes_dedupe_and_model_for() {
        let mut config = Config::default();
        config.llm.provider = "local".into();
        config.llm.lane = "cheap".into();
        config.llm.lanes = vec![
            LlmLane {
                name: "cheap".into(),
                base_url: "http://127.0.0.1:11434/v1".into(),
                model: "small".into(),
                api_key_env: None,
            },
            LlmLane {
                name: "strong".into(),
                base_url: "http://127.0.0.1:1234/v1".into(),
                model: "large".into(),
                api_key_env: None,
            },
        ];
        config.models = ModelsConfig {
            planning: "cheap".into(),
            coding: "strong".into(),
            review: "cheap".into(),
            debug: String::new(),
            summary: "cheap".into(),
        };
        config.validate().unwrap();

        let router = ModelRouter::from_config(&config, CancellationToken::new()).unwrap();
        assert_eq!(router.model_for(ModelRole::Coding), "large");
        assert_eq!(router.model_for(ModelRole::Planning), "small");
        assert_eq!(router.model_for(ModelRole::Review), "small");
        // empty models.debug → active lane "cheap"
        assert_eq!(router.lane_for(ModelRole::Debug), "cheap");
        assert_eq!(router.model_for(ModelRole::Debug), "small");

        // coding and planning are different providers when lanes differ
        assert!(!std::ptr::eq(
            Arc::as_ptr(&router.provider_for(ModelRole::Coding)) as *const (),
            Arc::as_ptr(&router.provider_for(ModelRole::Planning)) as *const ()
        ));
        // planning and review share the same lane → same Arc
        assert!(std::ptr::eq(
            Arc::as_ptr(&router.provider_for(ModelRole::Planning)) as *const (),
            Arc::as_ptr(&router.provider_for(ModelRole::Review)) as *const ()
        ));
    }

    #[test]
    fn from_config_rejects_unknown_provider() {
        let mut config = Config::default();
        config.llm.provider = "anthropic".into();
        // bypass Config::validate — router still rejects
        let err = ModelRouter::from_config(&config, CancellationToken::new());
        assert!(err.is_err());
    }
}
