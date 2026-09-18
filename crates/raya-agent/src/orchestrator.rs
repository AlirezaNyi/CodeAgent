//! Agent orchestration loop.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use raya_context::{ContextEngine, RankSignals};
use raya_core::{
    AgentDecision, AgentTask, CompletionRequest, Config, DagNode, DagNodeStatus, Event, EventKind,
    HeuristicCounter, MemoryKind, MemoryRecord, Message, TaskCheckpoint, TaskId, TaskPhase,
    TokenCounter, ToolCall, ToolCallId, VerificationStrategy, redact_secrets, validate_dag,
    write_plan_file,
};
use raya_index::{find_symbols, fts_search};
use raya_llm::{LlmProvider, ModelRole, ModelRouter};
use raya_store::Store;
use raya_tools::{ToolContext, ToolError, ToolRegistry};
use serde_json::json;
use thiserror::Error;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::resources::{ResourceLimits, ResourceManager};
use crate::subagent::{
    SubagentBrief, SubagentRole, SubagentRunner, SubagentVerdict, all_terminal, ready_nodes,
    skip_blocked,
};

const SYSTEM_PROMPT: &str = include_str!("../../../prompts/system.md");
const CHECKPOINT_MESSAGE_CAP: usize = 40;

fn build_rank_signals(store: &Store, root: &std::path::Path, request: &str) -> RankSignals {
    let mut signals = RankSignals::default();
    let keywords: Vec<String> = request
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_ascii_lowercase())
        .collect();

    for kw in keywords.iter().take(8) {
        if let Ok(syms) = find_symbols(store, kw, 20) {
            for s in syms {
                signals
                    .symbol_paths
                    .insert(s.path.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    if let Ok(paths) = fts_search(store, &keywords.join(" "), 30) {
        for p in paths {
            signals.fts_paths.insert(p);
        }
    }
    // Recent git files (best-effort, sync, short timeout via std::process)
    if let Ok(output) = std::process::Command::new("git")
        .args(["log", "--pretty=format:", "--name-only", "-n", "15"])
        .current_dir(root)
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let line = line.trim();
            if !line.is_empty() {
                signals.git_recent_paths.insert(line.replace('\\', "/"));
            }
        }
    }
    let _ = root;
    signals
}

#[derive(Debug, Error)]
pub enum OrchestratorError {
    #[error(transparent)]
    Store(#[from] raya_store::StoreError),

    #[error(transparent)]
    Llm(#[from] raya_llm::LlmError),

    #[error(transparent)]
    Context(#[from] raya_context::ContextError),

    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("cancelled")]
    Cancelled,

    #[error("resource limit: {0}")]
    Resource(String),

    #[error("{0}")]
    Message(String),
}

pub struct Orchestrator {
    store: Arc<Store>,
    tools: Arc<ToolRegistry>,
    router: Arc<ModelRouter>,
    config: Config,
    project_root: PathBuf,
}

impl Orchestrator {
    /// Compatibility constructor: wraps a single provider for all roles.
    pub fn new(
        store: Arc<Store>,
        tools: Arc<ToolRegistry>,
        llm: Arc<dyn LlmProvider>,
        config: Config,
        project_root: PathBuf,
    ) -> Self {
        Self::with_router(
            store,
            tools,
            Arc::new(ModelRouter::single(llm)),
            config,
            project_root,
        )
    }

    pub fn with_router(
        store: Arc<Store>,
        tools: Arc<ToolRegistry>,
        router: Arc<ModelRouter>,
        config: Config,
        project_root: PathBuf,
    ) -> Self {
        Self {
            store,
            tools,
            router,
            config,
            project_root,
        }
    }

    pub async fn run(
        &self,
        task_id: TaskId,
        cancel: CancellationToken,
    ) -> Result<AgentTask, OrchestratorError> {
        let mut task = self
            .store
            .get_task(task_id)?
            .ok_or_else(|| OrchestratorError::TaskNotFound(task_id.to_string()))?;

        let resources = Arc::new(ResourceManager::new(ResourceLimits {
            max_parallel_tools: self.config.resources.max_parallel_tools,
            max_parallel_agents: self.config.subagent.max_parallel,
            max_processes: self.config.resources.max_processes,
            max_iterations: task.max_iterations,
            max_tool_calls: task.max_tool_calls,
            max_tokens: task.max_tokens,
        }));

        let deadline = task.deadline.unwrap_or_else(|| {
            Utc::now() + chrono::Duration::seconds(self.config.agent.timeout_seconds as i64)
        });

        let mut messages: Vec<Message>;
        let mut pending_approval: Option<ToolCall>;
        let mut review_rounds: u32;
        let resuming = task.phase == TaskPhase::WaitingApproval;
        if !resuming && task.phase != TaskPhase::Created {
            return Err(OrchestratorError::Message(format!(
                "cannot run task in phase {phase:?}; expected created or waiting_approval",
                phase = task.phase
            )));
        }

        if resuming {
            let cp = self.store.load_checkpoint(task_id)?.ok_or_else(|| {
                OrchestratorError::Message(
                    "task is waiting_approval but no checkpoint was found; cannot resume".into(),
                )
            })?;
            messages = cp.messages;
            pending_approval = cp.pending_call;
            review_rounds = cp.review_rounds;
            self.store.append_event(&Event::new(
                task_id,
                EventKind::PhaseChanged,
                json!({"resumed": true, "phase": "waiting_approval"}),
            ))?;
            let _ = resources.add_tokens(task.tokens_used);
        } else {
            self.store
                .append_event(&Event::new(task_id, EventKind::TaskStarted, json!({})))?;

            task = self.transition(task, TaskPhase::Planning)?;

            if self.should_stop(&task, &cancel, deadline)? {
                return self.finish_cancelled(task).await;
            }
            let context_engine =
                ContextEngine::new(task.context_token_budget, self.config.context.max_files);
            task = self.transition(task, TaskPhase::ContextBuilding)?;

            let signals = build_rank_signals(&self.store, &self.project_root, &task.request);
            let bundle =
                context_engine.build_with_signals(&self.project_root, &task.request, &signals)?;
            let context_text = context_engine.render_prompt(&bundle);

            let (memory_block, memory_meta) = self.recall_memories(&task);
            let user_content = if memory_block.is_empty() {
                format!(
                    "Project root: {}\n\nRequest:\n{}\n\nRepository context:\n{}",
                    self.project_root.display(),
                    task.request,
                    context_text
                )
            } else {
                format!(
                    "Project root: {}\n\nRequest:\n{}\n\n{memory_block}\n\nRepository context:\n{}",
                    self.project_root.display(),
                    task.request,
                    context_text
                )
            };

            self.store.append_event(&Event::new(
                task_id,
                EventKind::ContextRetrieved,
                json!({
                    "files": bundle.snippets.iter().map(|s| s.path.display().to_string()).collect::<Vec<_>>(),
                    "tokens": bundle.total_tokens,
                    "keywords": bundle.keywords,
                    "explanations": bundle.snippets.iter().map(|s| json!({
                        "path": s.path.display().to_string(),
                        "notes": s.explanation.notes,
                        "lexical_hits": s.explanation.lexical_hits,
                        "path_match": s.explanation.path_match,
                        "symbol_match": s.explanation.symbol_match,
                        "git_recent": s.explanation.git_recent,
                        "fts_match": s.explanation.fts_match,
                    })).collect::<Vec<_>>(),
                    "memories": memory_meta,
                }),
            ))?;
            let _ = resources.add_tokens(bundle.total_tokens);

            task = self.transition(task, TaskPhase::Executing)?;

            messages = vec![Message::system(SYSTEM_PROMPT), Message::user(user_content)];
            pending_approval = None;
            review_rounds = 0;
        }

        loop {
            if self.should_stop(&task, &cancel, deadline)? {
                return self.finish_cancelled(task).await;
            }

            if task.phase == TaskPhase::WaitingApproval {
                if let Some(call) = pending_approval.clone() {
                    match self.store.approval_status(call.id)? {
                        Some(true) => {
                            self.store.append_event(&Event::new(
                                task_id,
                                EventKind::ApprovalGranted,
                                json!({"call_id": call.id.to_string()}),
                            ))?;
                            task = self.transition(task, TaskPhase::Executing)?;
                            match self
                                .exec_tool_approved(&task, &call, &cancel, &resources)
                                .await
                            {
                                Ok(result_msg) => {
                                    messages.push(Message::assistant(format!(
                                        "tool {} executed",
                                        call.name
                                    )));
                                    messages.push(Message::user(result_msg));
                                    pending_approval = None;
                                    let _ = self.store.delete_checkpoint(task_id);
                                }
                                Err(e) => {
                                    task.error = Some(e.to_string());
                                    return self.fail(task).await;
                                }
                            }
                        }
                        Some(false) => {
                            self.store.append_event(&Event::new(
                                task_id,
                                EventKind::ApprovalDenied,
                                json!({"call_id": call.id.to_string(), "tool": call.name}),
                            ))?;
                            messages.push(Message::user(format!(
                                "Tool {} was denied by the user. Try another approach or finish.",
                                call.name
                            )));
                            pending_approval = None;
                            let _ = self.store.delete_checkpoint(task_id);
                            task = self.transition(task, TaskPhase::Executing)?;
                        }
                        None => {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                            continue;
                        }
                    }
                } else {
                    task = self.transition(task, TaskPhase::Executing)?;
                }
            }

            let iter = resources
                .bump_iteration()
                .map_err(OrchestratorError::Resource)?;
            task.iterations = iter;
            self.store.update_task(&task)?;

            self.store.append_event(&Event::new(
                task_id,
                EventKind::LlmRequest,
                json!({
                    "iteration": iter,
                    "messages": messages.len(),
                    "role": ModelRole::Coding.as_str(),
                    "lane": self.router.lane_for(ModelRole::Coding),
                    "model": self.router.model_for(ModelRole::Coding),
                }),
            ))?;

            let response = self
                .router
                .provider_for(ModelRole::Coding)
                .complete(CompletionRequest {
                    model: self.router.model_for(ModelRole::Coding),
                    messages: messages.clone(),
                    tools: Some(self.tools.schemas()),
                    structured_json: true,
                    temperature: Some(0.2),
                    max_tokens: Some(4096),
                })
                .await;

            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    self.store.append_event(&Event::new(
                        task_id,
                        EventKind::LlmResponse,
                        json!({"error": redact_secrets(&e.to_string())}),
                    ))?;
                    task.error = Some(e.to_string());
                    return self.fail(task).await;
                }
            };

            let _ = resources.add_tokens(response.usage.total_tokens);
            task.tokens_used = resources.tokens_used();

            let decision = match AgentDecision::from_json(&response.content) {
                Ok(d) => {
                    self.store.append_event(&Event::new(
                        task_id,
                        EventKind::LlmResponse,
                        json!({"ok": true, "usage": response.usage}),
                    ))?;
                    d
                }
                Err(err) => {
                    self.store.append_event(&Event::new(
                        task_id,
                        EventKind::LlmResponse,
                        json!({"invalid": true, "error": err, "raw": redact_secrets(&response.content)}),
                    ))?;
                    messages.push(Message::assistant(response.content));
                    messages.push(Message::user(format!(
                        "Your previous output was invalid JSON ({err}). Reply with a valid AgentDecision JSON object."
                    )));
                    if iter >= task.max_iterations {
                        task.error = Some("invalid LLM output repeatedly".into());
                        return self.fail(task).await;
                    }
                    continue;
                }
            };

            messages.push(Message::assistant(response.content.clone()));

            match decision {
                AgentDecision::Plan { plan } => {
                    task.plan = Some(plan.clone());
                    self.store.update_task(&task)?;
                    if let Err(e) = write_plan_file(&self.project_root, &plan) {
                        warn!(error = %e, "failed to write .raya/PLAN.md");
                    }
                    self.store.append_event(&Event::new(
                        task_id,
                        EventKind::PlanCreated,
                        json!({
                            "plan": plan,
                            "plan_md": ".raya/PLAN.md",
                        }),
                    ))?;
                    self.record_decision_memory(&task, &plan);
                    if let Some(report) = self
                        .maybe_run_dag(&task, &plan.nodes, &resources, &cancel)
                        .await
                    {
                        messages.push(Message::user(report));
                    } else {
                        messages.push(Message::user(
                            "Plan recorded (also written to .raya/PLAN.md). Execute the next step with a tool_call or finish when done."
                                .to_string(),
                        ));
                    }
                }
                AgentDecision::ToolCall { mut call } => {
                    if call.id.as_uuid().is_nil() {
                        call.id = ToolCallId::new();
                    }
                    resources
                        .bump_tool_call()
                        .map_err(OrchestratorError::Resource)?;
                    task.tool_calls = resources.tool_calls();
                    self.store.update_task(&task)?;

                    match self.exec_tool(&task, &call, &cancel, &resources).await {
                        Ok(result_msg) => {
                            messages.push(Message::user(result_msg));
                        }
                        Err(OrchestratorError::Message(msg)) if msg.starts_with("approval:") => {
                            pending_approval = Some(call.clone());
                            self.store.append_event(&Event::new(
                                task_id,
                                EventKind::ApprovalRequested,
                                json!({
                                    "call_id": call.id.to_string(),
                                    "tool": call.name,
                                    "reason": msg,
                                }),
                            ))?;
                            task = self.transition(task, TaskPhase::WaitingApproval)?;
                            self.persist_checkpoint(
                                task_id,
                                &messages,
                                pending_approval.clone(),
                                review_rounds,
                            )?;
                            return Ok(task);
                        }
                        Err(e) => {
                            messages.push(Message::user(format!(
                                "Tool {} failed: {e}. Try another approach or finish.",
                                call.name
                            )));
                        }
                    }
                }
                AgentDecision::Delegate {
                    role,
                    objective,
                    paths,
                } => {
                    resources
                        .bump_tool_call()
                        .map_err(OrchestratorError::Resource)?;
                    task.tool_calls = resources.tool_calls();
                    self.store.update_task(&task)?;

                    let Some(sub_role) = SubagentRole::parse(&role) else {
                        messages.push(Message::user(format!(
                            "Unknown subagent role `{role}`. Use planner|coder|reviewer|debugger."
                        )));
                        continue;
                    };

                    let outcome = self
                        .run_subagent(
                            &task,
                            SubagentBrief::new(sub_role, objective).with_paths(paths),
                            &resources,
                            &cancel,
                        )
                        .await;
                    task.tokens_used = resources.tokens_used();
                    self.store.update_task(&task)?;

                    if let SubagentVerdict::Plan(plan) = &outcome.verdict {
                        task.plan = Some(plan.clone());
                        self.store.update_task(&task)?;
                        if let Err(e) = write_plan_file(&self.project_root, plan) {
                            warn!(error = %e, "failed to write .raya/PLAN.md");
                        }
                        self.store.append_event(&Event::new(
                            task_id,
                            EventKind::PlanCreated,
                            json!({
                                "plan": plan,
                                "plan_md": ".raya/PLAN.md",
                                "via": "subagent",
                                "role": sub_role.as_str(),
                            }),
                        ))?;
                        self.record_decision_memory(&task, plan);
                        if let Some(report) = self
                            .maybe_run_dag(&task, &plan.nodes, &resources, &cancel)
                            .await
                        {
                            messages.push(Message::user(format!(
                                "Subagent {} result (ok={}): {}\n\n{report}",
                                sub_role.as_str(),
                                outcome.ok,
                                outcome.summary
                            )));
                            continue;
                        }
                    }

                    messages.push(Message::user(format!(
                        "Subagent {} result (ok={}): {}\nContinue with tool_call, delegate, or finish.",
                        sub_role.as_str(),
                        outcome.ok,
                        outcome.summary
                    )));
                }
                AgentDecision::Finish { summary } => {
                    task = self.transition(task, TaskPhase::Verifying)?;
                    let (verify_ok, verify_detail) = self.verify(&task, &cancel).await?;
                    if verify_ok {
                        let mut final_summary = summary.clone();
                        if self.config.subagent.review_on_finish {
                            let review = self
                                .run_subagent(
                                    &task,
                                    SubagentBrief::new(
                                        SubagentRole::Reviewer,
                                        format!("Review completed work: {summary}"),
                                    )
                                    .with_extra(git_diff_snippet(&self.project_root)),
                                    &resources,
                                    &cancel,
                                )
                                .await;
                            task.tokens_used = resources.tokens_used();
                            match review.verdict {
                                SubagentVerdict::NeedsFix { reason }
                                    if review_rounds < self.config.subagent.max_review_rounds =>
                                {
                                    review_rounds += 1;
                                    task = self.transition(task, TaskPhase::Fixing)?;
                                    messages.push(Message::user(format!(
                                        "Reviewer requested fixes: {reason}. Continue with tool_call, then finish again."
                                    )));
                                    task = self.transition(task, TaskPhase::Executing)?;
                                    continue;
                                }
                                SubagentVerdict::NeedsFix { reason } => {
                                    warn!(
                                        %reason,
                                        review_rounds,
                                        "reviewer rejected after max rounds; completing anyway"
                                    );
                                    final_summary =
                                        format!("{summary} (reviewer warning: {reason})");
                                }
                                _ => {}
                            }
                        }
                        info!(summary = %final_summary, "task completed");
                        task.current_step = Some(final_summary.clone());
                        self.store.update_task(&task)?;
                        self.record_task_memory(&task, &final_summary);
                        let _ = self.store.delete_checkpoint(task_id);
                        let _ = self.store.delete_dag(task_id);
                        return self.transition(task, TaskPhase::Completed);
                    }

                    if self.config.subagent.debug_on_verify_fail {
                        let debug = self
                            .run_subagent(
                                &task,
                                SubagentBrief::new(
                                    SubagentRole::Debugger,
                                    "Diagnose verification failure".to_string(),
                                )
                                .with_extra(verify_detail.clone()),
                                &resources,
                                &cancel,
                            )
                            .await;
                        task.tokens_used = resources.tokens_used();
                        messages.push(Message::user(format!(
                            "Debugger report (ok={}): {}\nVerification output:\n{}",
                            debug.ok, debug.summary, verify_detail
                        )));
                    }

                    task = self.transition(task, TaskPhase::Fixing)?;
                    messages.push(Message::user(
                        "Verification failed. Diagnose and fix with tool_call, then finish again."
                            .to_string(),
                    ));
                    task = self.transition(task, TaskPhase::Executing)?;
                }
                AgentDecision::NeedsFix { reason } => {
                    task = self.transition(task, TaskPhase::Fixing)?;
                    messages.push(Message::user(format!(
                        "Fix required: {reason}. Continue with tool_call."
                    )));
                    task = self.transition(task, TaskPhase::Executing)?;
                }
            }
        }
    }

    fn persist_checkpoint(
        &self,
        task_id: TaskId,
        messages: &[Message],
        pending_call: Option<ToolCall>,
        review_rounds: u32,
    ) -> Result<(), OrchestratorError> {
        let start = messages.len().saturating_sub(CHECKPOINT_MESSAGE_CAP);
        let capped: Vec<Message> = messages[start..]
            .iter()
            .map(|m| Message {
                role: m.role,
                content: redact_secrets(&m.content),
                name: m.name.clone(),
                tool_call_id: m.tool_call_id.clone(),
            })
            .collect();
        let pending = pending_call.map(|mut call| {
            call.input = redact_json_value(&call.input);
            call
        });
        let mut cp = TaskCheckpoint::new(task_id, capped, pending);
        cp.review_rounds = review_rounds;
        self.store.save_checkpoint(&cp)?;
        Ok(())
    }

    fn recall_memories(&self, task: &AgentTask) -> (String, Vec<serde_json::Value>) {
        if !self.config.memory.enabled {
            return (String::new(), Vec::new());
        }
        let Ok(rows) = self.store.search_memories(
            task.project_id,
            &task.request,
            self.config.memory.max_items,
        ) else {
            return (String::new(), Vec::new());
        };
        if rows.is_empty() {
            return (String::new(), Vec::new());
        }
        let counter = HeuristicCounter;
        let mut block = String::from("## Project memory\n");
        let mut used = counter.count(&block);
        let mut meta = Vec::new();
        for m in rows {
            let entry = format!("- [{}] {}\n", m.kind.as_str(), m.content);
            let t = counter.count(&entry);
            if used + t > self.config.memory.max_tokens {
                break;
            }
            block.push_str(&entry);
            used += t;
            meta.push(json!({"id": m.id, "kind": m.kind.as_str()}));
        }
        if meta.is_empty() {
            (String::new(), Vec::new())
        } else {
            (block, meta)
        }
    }

    fn record_task_memory(&self, task: &AgentTask, summary: &str) {
        if !self.config.memory.enabled {
            return;
        }
        let files: Vec<String> = self
            .store
            .list_events(task.id, None, 200)
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, e)| e.kind == EventKind::FileModified)
            .filter_map(|(_, e)| {
                e.payload
                    .get("input")
                    .and_then(|v| v.get("path"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        let content = if files.is_empty() {
            format!("Request: {}\nSummary: {summary}", task.request)
        } else {
            format!(
                "Request: {}\nSummary: {summary}\nFiles: {}",
                task.request,
                files.join(", ")
            )
        };
        let mem = MemoryRecord::new(
            task.project_id,
            MemoryKind::Task,
            redact_secrets(&content),
            Some(task.id),
        )
        .with_key("task_summary");
        if let Err(e) = self.store.upsert_memory(&mem) {
            warn!(error = %e, "failed to record task memory");
        }
    }

    fn record_decision_memory(&self, task: &AgentTask, plan: &raya_core::ExecutionPlan) {
        if !self.config.memory.enabled {
            return;
        }
        let steps: Vec<String> = plan
            .steps
            .iter()
            .map(|s| format!("{}: {}", s.id, s.description))
            .collect();
        let summary = plan.summary.clone().unwrap_or_default();
        let content = redact_secrets(&format!(
            "Plan summary: {summary}\nSteps:\n{}",
            steps.join("\n")
        ));
        let mem = MemoryRecord::new(
            task.project_id,
            MemoryKind::Decision,
            content,
            Some(task.id),
        )
        .with_key("plan");
        if let Err(e) = self.store.upsert_memory(&mem) {
            warn!(error = %e, "failed to record decision memory");
        }
    }

    /// Run optional plan DAG if nodes are non-empty and valid.
    /// Returns a user-message report, or None when DAG is skipped.
    async fn maybe_run_dag(
        &self,
        task: &AgentTask,
        nodes: &[DagNode],
        resources: &Arc<ResourceManager>,
        cancel: &CancellationToken,
    ) -> Option<String> {
        if nodes.is_empty() {
            return None;
        }
        if let Err(e) = validate_dag(nodes, self.config.subagent.max_dag_nodes) {
            warn!(error = %e, "invalid plan DAG; skipping scheduler");
            return Some(format!(
                "Plan recorded, but DAG was skipped ({e}). Continue with tool_call or finish."
            ));
        }
        match self.run_plan_dag(task, nodes, resources, cancel).await {
            Ok(report) => Some(report),
            Err(e) => {
                warn!(error = %e, "DAG execution failed");
                Some(format!(
                    "Plan recorded, but DAG failed ({e}). Continue with tool_call or finish."
                ))
            }
        }
    }

    async fn run_plan_dag(
        &self,
        task: &AgentTask,
        nodes: &[DagNode],
        resources: &Arc<ResourceManager>,
        cancel: &CancellationToken,
    ) -> Result<String, OrchestratorError> {
        self.store.replace_dag(task.id, nodes)?;
        let mut status: std::collections::HashMap<String, DagNodeStatus> = nodes
            .iter()
            .map(|n| (n.id.clone(), DagNodeStatus::Pending))
            .collect();
        let mut summaries: std::collections::HashMap<String, (bool, String, String)> =
            std::collections::HashMap::new();

        while !all_terminal(&status) {
            if cancel.is_cancelled() || self.store.is_cancel_requested(task.id).unwrap_or(false) {
                return Err(OrchestratorError::Cancelled);
            }

            let ready = ready_nodes(nodes, &status);
            if ready.is_empty() {
                let skipped = skip_blocked(nodes, &mut status);
                for id in &skipped {
                    let _ = self.store.set_dag_node_status(
                        task.id,
                        id,
                        DagNodeStatus::Skipped,
                        Some("skipped due to failed dependency"),
                    );
                    summaries.insert(
                        id.clone(),
                        (
                            false,
                            "skipped".into(),
                            "skipped due to failed dependency".into(),
                        ),
                    );
                }
                if skipped.is_empty() && !all_terminal(&status) {
                    // Deadlock safety: mark remaining pending as skipped.
                    for (id, st) in status.iter_mut() {
                        if *st == DagNodeStatus::Pending {
                            *st = DagNodeStatus::Skipped;
                            let _ = self.store.set_dag_node_status(
                                task.id,
                                id,
                                DagNodeStatus::Skipped,
                                Some("skipped (unreachable)"),
                            );
                            summaries.insert(
                                id.clone(),
                                (false, "skipped".into(), "unreachable".into()),
                            );
                        }
                    }
                }
                continue;
            }

            let mut set = JoinSet::new();
            for node in ready {
                let _ =
                    self.store
                        .set_dag_node_status(task.id, &node.id, DagNodeStatus::Running, None);
                status.insert(node.id.clone(), DagNodeStatus::Running);

                let role = SubagentRole::parse(&node.role).ok_or_else(|| {
                    OrchestratorError::Message(format!("invalid dag role {}", node.role))
                })?;
                let brief = SubagentBrief::new(role, node.objective.clone())
                    .with_paths(node.paths.clone())
                    .with_dag_node(node.id.clone());

                let store = self.store.clone();
                let tools = self.tools.clone();
                let router = self.router.clone();
                let config = self.config.clone();
                let root = self.project_root.clone();
                let task_clone = task.clone();
                let resources = resources.clone();
                let cancel = cancel.clone();
                let node_id = node.id.clone();
                let role_str = role.as_str().to_string();

                set.spawn(async move {
                    let runner = SubagentRunner {
                        store: store.as_ref(),
                        tools: tools.as_ref(),
                        router: router.as_ref(),
                        config: &config,
                        project_root: &root,
                        task: &task_clone,
                    };
                    let outcome = runner.run(brief, resources.as_ref(), &cancel).await;
                    (node_id, role_str, outcome)
                });
            }

            while let Some(joined) = set.join_next().await {
                let (node_id, role_str, outcome) = joined
                    .map_err(|e| OrchestratorError::Message(format!("dag join error: {e}")))?;
                let st = if outcome.ok {
                    DagNodeStatus::Completed
                } else {
                    DagNodeStatus::Failed
                };
                let _ = self.store.set_dag_node_status(
                    task.id,
                    &node_id,
                    st,
                    Some(outcome.summary.as_str()),
                );
                status.insert(node_id.clone(), st);
                summaries.insert(node_id, (outcome.ok, role_str, outcome.summary));
            }
        }

        let mut lines = vec!["DAG completed:".to_string()];
        for n in nodes {
            let (ok, role, summary) = summaries.get(&n.id).cloned().unwrap_or((
                false,
                n.role.clone(),
                "no result".into(),
            ));
            lines.push(format!("- [{}] {role} ok={ok}: {summary}", n.id));
        }
        lines.push("Continue with tool_call, delegate, or finish.".into());
        Ok(lines.join("\n"))
    }

    async fn run_subagent(
        &self,
        task: &AgentTask,
        brief: SubagentBrief,
        resources: &ResourceManager,
        cancel: &CancellationToken,
    ) -> crate::subagent::SubagentOutcome {
        let runner = SubagentRunner {
            store: self.store.as_ref(),
            tools: self.tools.as_ref(),
            router: self.router.as_ref(),
            config: &self.config,
            project_root: &self.project_root,
            task,
        };
        runner.run(brief, resources, cancel).await
    }

    async fn exec_tool(
        &self,
        task: &AgentTask,
        call: &ToolCall,
        cancel: &CancellationToken,
        resources: &ResourceManager,
    ) -> Result<String, OrchestratorError> {
        self.exec_tool_inner(task, call, cancel, resources, false)
            .await
    }

    async fn exec_tool_approved(
        &self,
        task: &AgentTask,
        call: &ToolCall,
        cancel: &CancellationToken,
        resources: &ResourceManager,
    ) -> Result<String, OrchestratorError> {
        self.exec_tool_inner(task, call, cancel, resources, true)
            .await
    }

    async fn exec_tool_inner(
        &self,
        task: &AgentTask,
        call: &ToolCall,
        cancel: &CancellationToken,
        resources: &ResourceManager,
        already_approved: bool,
    ) -> Result<String, OrchestratorError> {
        let _permit = resources
            .acquire_tool()
            .await
            .map_err(OrchestratorError::Resource)?;

        self.store.append_event(&Event::new(
            task.id,
            EventKind::ToolStarted,
            json!({"name": call.name, "call_id": call.id.to_string()}),
        ))?;

        let ctx = ToolContext::new(task.id, self.project_root.clone(), cancel.clone());
        let result = if already_approved {
            self.tools.execute_approved(&ctx, call).await
        } else {
            self.tools.execute(&ctx, call).await
        };
        match result {
            Ok(result) => {
                self.store.append_event(&Event::new(
                    task.id,
                    EventKind::ToolCompleted,
                    json!({
                        "name": result.name,
                        "success": result.success,
                        "truncated": result.truncated,
                        "duration_ms": result.duration_ms,
                    }),
                ))?;
                if call.name.starts_with("filesystem.") {
                    self.store.append_event(&Event::new(
                        task.id,
                        EventKind::FileModified,
                        json!({"tool": call.name, "input": call.input}),
                    ))?;
                }
                Ok(format!(
                    "Tool {} result (success={}):\n{}",
                    result.name,
                    result.success,
                    redact_secrets(&result.output)
                ))
            }
            Err(ToolError::ApprovalRequired(reason)) => {
                Err(OrchestratorError::Message(format!("approval:{reason}")))
            }
            Err(ToolError::Denied(reason)) => {
                self.store.append_event(&Event::new(
                    task.id,
                    EventKind::ToolCompleted,
                    json!({"name": call.name, "denied": true, "reason": reason}),
                ))?;
                Ok(format!("Tool {} denied by policy: {reason}", call.name))
            }
            Err(ToolError::Cancelled) => Err(OrchestratorError::Cancelled),
            Err(e) => Ok(format!("Tool {} error: {e}", call.name)),
        }
    }

    async fn verify(
        &self,
        task: &AgentTask,
        cancel: &CancellationToken,
    ) -> Result<(bool, String), OrchestratorError> {
        let strategy = task
            .plan
            .as_ref()
            .map(|p| p.verification.clone())
            .unwrap_or(VerificationStrategy::None);

        let command = match strategy {
            VerificationStrategy::None => return Ok((true, String::new())),
            VerificationStrategy::Test => "test.run",
            VerificationStrategy::Build => "build.run",
            VerificationStrategy::TestAndBuild => "test.run",
            VerificationStrategy::Custom { .. } => "shell.exec",
        };

        self.store.append_event(&Event::new(
            task.id,
            EventKind::TestStarted,
            json!({"tool": command}),
        ))?;

        let ctx = ToolContext::new(task.id, self.project_root.clone(), cancel.clone());
        let input = match &strategy {
            VerificationStrategy::Custom { command } => json!({"command": command}),
            _ => json!({}),
        };

        let result = match self.tools.execute_named(&ctx, command, input).await {
            Ok(r) => r,
            Err(ToolError::Denied(reason)) | Err(ToolError::ApprovalRequired(reason)) => {
                warn!(%reason, "verification skipped due to policy");
                return Ok((true, format!("skipped: {reason}")));
            }
            Err(e) => {
                let detail = e.to_string();
                self.store.append_event(&Event::new(
                    task.id,
                    EventKind::TestFailed,
                    json!({"error": detail.clone()}),
                ))?;
                return Ok((false, detail));
            }
        };

        if result.success {
            self.store
                .append_event(&Event::new(task.id, EventKind::TestPassed, json!({})))?;
            Ok((true, result.output))
        } else {
            let detail = redact_secrets(&result.output);
            self.store.append_event(&Event::new(
                task.id,
                EventKind::TestFailed,
                json!({"output": detail.clone()}),
            ))?;
            Ok((false, detail))
        }
    }

    fn transition(&self, task: AgentTask, to: TaskPhase) -> Result<AgentTask, OrchestratorError> {
        Ok(self.store.transition_task(task.id, task.phase, to)?)
    }

    fn should_stop(
        &self,
        task: &AgentTask,
        cancel: &CancellationToken,
        deadline: chrono::DateTime<Utc>,
    ) -> Result<bool, OrchestratorError> {
        if cancel.is_cancelled() || self.store.is_cancel_requested(task.id)? {
            return Ok(true);
        }
        if Utc::now() > deadline {
            return Ok(true);
        }
        Ok(false)
    }

    async fn finish_cancelled(&self, mut task: AgentTask) -> Result<AgentTask, OrchestratorError> {
        let _ = self.store.delete_checkpoint(task.id);
        let _ = self.store.delete_dag(task.id);
        if !task.phase.is_terminal() {
            if task.phase.can_transition_to(TaskPhase::Cancelled) {
                task = self.transition(task, TaskPhase::Cancelled)?;
            } else {
                task.phase = TaskPhase::Cancelled;
                task.updated_at = Utc::now();
                self.store.update_task(&task)?;
                self.store.append_event(&Event::new(
                    task.id,
                    EventKind::TaskCancelled,
                    json!({}),
                ))?;
            }
        }
        Ok(task)
    }

    async fn fail(&self, mut task: AgentTask) -> Result<AgentTask, OrchestratorError> {
        let _ = self.store.delete_checkpoint(task.id);
        let _ = self.store.delete_dag(task.id);
        if task.phase.can_transition_to(TaskPhase::Failed) {
            task = self.transition(task, TaskPhase::Failed)?;
        } else {
            task.phase = TaskPhase::Failed;
            self.store.update_task(&task)?;
            self.store.append_event(&Event::new(
                task.id,
                EventKind::TaskFailed,
                json!({"error": task.error}),
            ))?;
        }
        Ok(task)
    }
}

fn redact_json_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(redact_secrets(s)),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_json_value).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), redact_json_value(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn git_diff_snippet(root: &std::path::Path) -> String {
    match std::process::Command::new("git")
        .args(["diff", "--stat", "HEAD"])
        .current_dir(root)
        .output()
    {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            if text.trim().is_empty() {
                "(no git diff)".into()
            } else {
                text.chars().take(4000).collect()
            }
        }
        Err(_) => "(git diff unavailable)".into(),
    }
}
