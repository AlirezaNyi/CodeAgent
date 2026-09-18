//! Bounded subagent execution loop.

use std::path::Path;
use std::time::Instant;

use raya_core::{
    AgentDecision, AgentTask, CompletionRequest, Config, Event, EventKind, HeuristicCounter,
    Message, TaskId, TokenCounter, ToolCallId, redact_secrets,
};
use raya_llm::ModelRouter;
use raya_store::Store;
use raya_tools::{ToolContext, ToolError, ToolRegistry, safe_path};
use serde_json::json;
use tokio::time::{Duration, timeout};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use crate::resources::{ResourceLimits, ResourceManager};

use super::brief::{SubagentBrief, SubagentOutcome, SubagentVerdict};
use super::role::SubagentRole;

/// Runs a single ephemeral subagent under parent resource limits.
pub struct SubagentRunner<'a> {
    pub store: &'a Store,
    pub tools: &'a ToolRegistry,
    pub router: &'a ModelRouter,
    pub config: &'a Config,
    pub project_root: &'a Path,
    pub task: &'a AgentTask,
}

impl<'a> SubagentRunner<'a> {
    pub async fn run(
        &self,
        brief: SubagentBrief,
        parent: &ResourceManager,
        cancel: &CancellationToken,
    ) -> SubagentOutcome {
        let start = Instant::now();
        let agent_id = Uuid::new_v4().to_string();
        let role = brief.role;
        let model_role = role.model_role();

        let permit = match parent.acquire_agent().await {
            Ok(p) => p,
            Err(e) => {
                return self.fail_outcome(
                    &agent_id,
                    role,
                    format!("failed to acquire agent slot: {e}"),
                    start,
                );
            }
        };

        let mut spawned = json!({
            "agent_id": agent_id,
            "role": role.as_str(),
            "objective": brief.objective,
            "lane": self.router.lane_for(model_role),
            "model": self.router.model_for(model_role),
        });
        if let Some(node_id) = &brief.dag_node_id {
            spawned["dag_node_id"] = json!(node_id);
        }
        let _ =
            self.store
                .append_event(&Event::new(self.task.id, EventKind::AgentSpawned, spawned));

        let child = cancel.child_token();
        let timeout_secs = self.config.subagent.timeout_seconds;
        let fut = self.run_inner(&brief, &agent_id, parent, &child);
        let outcome = match timeout(Duration::from_secs(timeout_secs), fut).await {
            Ok(o) => o,
            Err(_) => {
                child.cancel();
                self.fail_outcome(&agent_id, role, "subagent timed out".into(), start)
            }
        };

        drop(permit);

        let mut completed = json!({
            "agent_id": outcome.agent_id,
            "role": outcome.role.as_str(),
            "ok": outcome.ok,
            "summary": outcome.summary,
            "tool_calls": outcome.tool_calls,
            "tokens": outcome.tokens_used,
            "duration_ms": outcome.duration_ms,
        });
        if let Some(node_id) = &brief.dag_node_id {
            completed["dag_node_id"] = json!(node_id);
        }
        let _ = self.store.append_event(&Event::new(
            self.task.id,
            EventKind::AgentCompleted,
            completed,
        ));

        if outcome.tokens_used > 0 {
            let _ = parent.add_tokens(outcome.tokens_used);
        }

        outcome
    }

    async fn run_inner(
        &self,
        brief: &SubagentBrief,
        agent_id: &str,
        parent: &ResourceManager,
        cancel: &CancellationToken,
    ) -> SubagentOutcome {
        let start = Instant::now();
        let role = brief.role;
        let model_role = role.model_role();

        let remaining = self
            .task
            .max_tokens
            .saturating_sub(parent.tokens_used())
            .max(1);
        let child_token_cap = remaining.min(self.task.context_token_budget.max(1));

        let child_rm = ResourceManager::new(ResourceLimits {
            max_parallel_tools: self.config.resources.max_parallel_tools,
            max_parallel_agents: 1,
            max_processes: self.config.resources.max_processes,
            max_iterations: self.config.subagent.max_iterations,
            max_tool_calls: self.config.subagent.max_tool_calls,
            max_tokens: child_token_cap,
        });

        let context_text = self.build_minimal_context(brief);
        let provider = self.router.provider_for(model_role);
        let model = self.router.model_for(model_role);

        let allowed: Vec<_> = role.allowed_tools().to_vec();
        let schemas: Vec<_> = self
            .tools
            .schemas()
            .into_iter()
            .filter(|s| allowed.contains(&s.name.as_str()))
            .collect();

        let mut messages = vec![
            Message::system(role.prompt().to_string()),
            Message::user(format!(
                "Project root: {}\n\nObjective:\n{}\n\nContext:\n{}",
                self.project_root.display(),
                brief.objective,
                context_text
            )),
        ];

        let mut last_summary = String::new();
        let mut verdict = SubagentVerdict::Report;

        loop {
            if cancel.is_cancelled()
                || self
                    .store
                    .is_cancel_requested(self.task.id)
                    .unwrap_or(false)
            {
                return self.fail_outcome(agent_id, role, "cancelled".into(), start);
            }

            let iter = match child_rm.bump_iteration() {
                Ok(n) => n,
                Err(e) => {
                    return SubagentOutcome {
                        agent_id: agent_id.to_string(),
                        role,
                        ok: !last_summary.is_empty(),
                        summary: if last_summary.is_empty() {
                            e
                        } else {
                            last_summary
                        },
                        verdict,
                        iterations: child_rm.iterations(),
                        tool_calls: child_rm.tool_calls(),
                        tokens_used: child_rm.tokens_used(),
                        duration_ms: start.elapsed().as_millis() as u64,
                    };
                }
            };

            let response = match provider
                .complete(CompletionRequest {
                    model: model.clone(),
                    messages: messages.clone(),
                    tools: Some(schemas.clone()),
                    structured_json: true,
                    temperature: Some(0.2),
                    max_tokens: Some(2048),
                })
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    return self.fail_outcome(
                        agent_id,
                        role,
                        format!("llm error: {}", redact_secrets(&e.to_string())),
                        start,
                    );
                }
            };

            let _ = child_rm.add_tokens(response.usage.total_tokens);

            let decision = match AgentDecision::from_json(&response.content) {
                Ok(d) => d,
                Err(err) => {
                    messages.push(Message::assistant(response.content));
                    messages.push(Message::user(format!(
                        "Your previous output was invalid JSON ({err}). Reply with a valid AgentDecision JSON object."
                    )));
                    if iter >= self.config.subagent.max_iterations {
                        return self.fail_outcome(
                            agent_id,
                            role,
                            "invalid LLM output repeatedly".into(),
                            start,
                        );
                    }
                    continue;
                }
            };

            messages.push(Message::assistant(response.content.clone()));

            match decision {
                AgentDecision::Plan { plan } => {
                    last_summary = plan
                        .summary
                        .clone()
                        .unwrap_or_else(|| "plan produced".into());
                    verdict = SubagentVerdict::Plan(plan);
                    return SubagentOutcome {
                        agent_id: agent_id.to_string(),
                        role,
                        ok: true,
                        summary: last_summary,
                        verdict,
                        iterations: child_rm.iterations(),
                        tool_calls: child_rm.tool_calls(),
                        tokens_used: child_rm.tokens_used(),
                        duration_ms: start.elapsed().as_millis() as u64,
                    };
                }
                AgentDecision::ToolCall { mut call } => {
                    if call.id.as_uuid().is_nil() {
                        call.id = ToolCallId::new();
                    }
                    if let Err(e) = child_rm.bump_tool_call() {
                        return self.fail_outcome(agent_id, role, e, start);
                    }
                    if !role.permits_tool(&call.name) {
                        messages.push(Message::user(format!(
                            "Tool {} is not permitted for role {}. Choose another tool or finish.",
                            call.name,
                            role.as_str()
                        )));
                        continue;
                    }

                    let msg = self.exec_tool(self.task.id, &call, cancel, &child_rm).await;
                    messages.push(Message::user(msg));
                }
                AgentDecision::Finish { summary } => {
                    last_summary = summary;
                    verdict = match role {
                        SubagentRole::Reviewer => SubagentVerdict::Approve,
                        _ => SubagentVerdict::Report,
                    };
                    info!(agent_id = %agent_id, role = role.as_str(), "subagent finished");
                    return SubagentOutcome {
                        agent_id: agent_id.to_string(),
                        role,
                        ok: true,
                        summary: last_summary,
                        verdict,
                        iterations: child_rm.iterations(),
                        tool_calls: child_rm.tool_calls(),
                        tokens_used: child_rm.tokens_used(),
                        duration_ms: start.elapsed().as_millis() as u64,
                    };
                }
                AgentDecision::NeedsFix { reason } => {
                    last_summary = reason.clone();
                    verdict = SubagentVerdict::NeedsFix { reason };
                    return SubagentOutcome {
                        agent_id: agent_id.to_string(),
                        role,
                        ok: true,
                        summary: last_summary,
                        verdict,
                        iterations: child_rm.iterations(),
                        tool_calls: child_rm.tool_calls(),
                        tokens_used: child_rm.tokens_used(),
                        duration_ms: start.elapsed().as_millis() as u64,
                    };
                }
                AgentDecision::Delegate { role: r, .. } => {
                    messages.push(Message::user(format!(
                        "Subagents cannot delegate further (requested {r}). Use tool_call or finish."
                    )));
                }
            }
        }
    }

    async fn exec_tool(
        &self,
        task_id: TaskId,
        call: &raya_core::ToolCall,
        cancel: &CancellationToken,
        resources: &ResourceManager,
    ) -> String {
        let _permit = match resources.acquire_tool().await {
            Ok(p) => p,
            Err(e) => return format!("Tool concurrency error: {e}"),
        };

        let ctx = ToolContext::new(task_id, self.project_root.to_path_buf(), cancel.clone());
        match self.tools.execute(&ctx, call).await {
            Ok(result) => format!(
                "Tool {} result (success={}):\n{}",
                result.name,
                result.success,
                redact_secrets(&result.output)
            ),
            Err(ToolError::ApprovalRequired(reason)) => {
                // Subagents cannot pause the parent for approval.
                format!(
                    "Tool {} requires approval and cannot run inside a subagent: {reason}",
                    call.name
                )
            }
            Err(ToolError::Denied(reason)) => {
                format!("Tool {} denied by policy: {reason}", call.name)
            }
            Err(ToolError::Cancelled) => "Tool cancelled".into(),
            Err(e) => format!("Tool {} error: {e}", call.name),
        }
    }

    fn build_minimal_context(&self, brief: &SubagentBrief) -> String {
        let counter = HeuristicCounter;
        let max_tokens = self.task.context_token_budget.min(8_000);
        let max_files = self.config.context.max_files.min(20) as usize;
        let mut out = String::new();
        let mut used = 0u64;

        if !brief.extra_context.is_empty() {
            let t = counter.count(&brief.extra_context);
            if t <= max_tokens {
                out.push_str("## Extra\n");
                out.push_str(&brief.extra_context);
                out.push('\n');
                used += t;
            }
        }

        for (i, rel) in brief.context_paths.iter().enumerate() {
            if i >= max_files {
                break;
            }
            let Ok(path) = safe_path(self.project_root, rel) else {
                continue;
            };
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let snippet = if content.len() > 12_000 {
                format!("{}\n…(truncated)…\n", &content[..12_000])
            } else {
                content
            };
            let block = format!("## File: {rel}\n```\n{snippet}\n```\n");
            let t = counter.count(&block);
            if used + t > max_tokens {
                break;
            }
            out.push_str(&block);
            used += t;
        }

        if out.is_empty() {
            "(no additional context)".into()
        } else {
            out
        }
    }

    fn fail_outcome(
        &self,
        agent_id: &str,
        role: SubagentRole,
        summary: String,
        start: Instant,
    ) -> SubagentOutcome {
        warn!(agent_id = %agent_id, role = role.as_str(), %summary, "subagent failed");
        SubagentOutcome {
            agent_id: agent_id.to_string(),
            role,
            ok: false,
            summary,
            verdict: SubagentVerdict::Report,
            iterations: 0,
            tool_calls: 0,
            tokens_used: 0,
            duration_ms: start.elapsed().as_millis() as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raya_core::PolicyAction;
    use raya_llm::MockProvider;
    use raya_policy::PolicyEngine;
    use raya_tools::default_registry;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, Arc<Store>, Config, Arc<ToolRegistry>) {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".raya")).unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
        let store = Arc::new(Store::open(dir.path().join(".raya/raya.db")).unwrap());
        let mut config = Config::default();
        config.llm.provider = "mock".into();
        config.policy.shell = PolicyAction::Deny;
        let tools = Arc::new(default_registry(PolicyEngine::new(config.policy.clone())));
        (dir, store, config, tools)
    }

    #[tokio::test]
    async fn rejects_disallowed_tool_without_executing() {
        let (dir, store, config, tools) = setup();
        let project = store.get_or_create_project(dir.path(), "demo").unwrap();
        let task = AgentTask::new(project.id, "t", 12, 40, 100_000, 40_000, None);
        store.create_task(&task).unwrap();

        // Script: try shell.exec (disallowed), then finish
        let mock = MockProvider::new(vec![
            raya_core::CompletionResponse {
                content: json!({
                    "type": "tool_call",
                    "call": {
                        "id": "00000000-0000-4000-8000-000000000099",
                        "name": "shell.exec",
                        "input": {"command": "echo hi"}
                    }
                })
                .to_string(),
                structured: None,
                usage: raya_core::TokenUsage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: 2,
                },
                model: Some("mock".into()),
                finish_reason: Some("stop".into()),
            },
            raya_core::CompletionResponse {
                content: json!({"type":"finish","summary":"done without shell"}).to_string(),
                structured: None,
                usage: raya_core::TokenUsage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: 2,
                },
                model: Some("mock".into()),
                finish_reason: Some("stop".into()),
            },
        ]);
        let router = ModelRouter::single(Arc::new(mock));
        let parent = ResourceManager::new(ResourceLimits::from_config(&config));
        let runner = SubagentRunner {
            store: &store,
            tools: &tools,
            router: &router,
            config: &config,
            project_root: dir.path(),
            task: &task,
        };
        let outcome = runner
            .run(
                SubagentBrief::new(SubagentRole::Reviewer, "review"),
                &parent,
                &CancellationToken::new(),
            )
            .await;
        assert!(outcome.ok);
        assert_eq!(outcome.summary, "done without shell");
        // shell never completed as a tool event from registry for that call —
        // only agent.spawned/completed should exist for this path
        let events = store.list_events(task.id, None, 100).unwrap();
        let kinds: Vec<_> = events.iter().map(|(_, e)| e.kind.as_str()).collect();
        assert!(kinds.contains(&"agent.spawned"));
        assert!(kinds.contains(&"agent.completed"));
    }

    #[tokio::test]
    async fn cancel_yields_not_ok() {
        let (dir, store, config, tools) = setup();
        let project = store.get_or_create_project(dir.path(), "demo").unwrap();
        let task = AgentTask::new(project.id, "t", 12, 40, 100_000, 40_000, None);
        store.create_task(&task).unwrap();

        // Empty mock will error on first complete — use a cancelled token instead
        let router = ModelRouter::single(Arc::new(MockProvider::default_script()));
        let parent = ResourceManager::new(ResourceLimits::from_config(&config));
        let cancel = CancellationToken::new();
        cancel.cancel();
        let runner = SubagentRunner {
            store: &store,
            tools: &tools,
            router: &router,
            config: &config,
            project_root: dir.path(),
            task: &task,
        };
        let outcome = runner
            .run(
                SubagentBrief::new(SubagentRole::Debugger, "debug"),
                &parent,
                &cancel,
            )
            .await;
        assert!(!outcome.ok);
    }
}
