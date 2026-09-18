# ADR 0011: Phase 3 subagents and model router

## Status
Accepted

## Context
Phase 1–2 delivered the vertical slice, index/ranking, and named LLM lanes (ADR 0010). Phase 3 needs bounded ephemeral workers (Planner / Coder / Reviewer / Debugger), a role→lane model router, and an optional LLM `delegate` decision — without shipping Memory, MCP, or a full execution DAG yet.

## Decision
1. **In-process module, not a new crate.** Subagents live in `crates/raya-agent/src/subagent/`. Extract to `raya-subagent` later if MCP/Phase 4 needs a separate dependency boundary.
2. **Model router** (`raya-llm::ModelRouter`) maps `ModelRole` {planning, coding, review, debug, summary} → named lane / provider. Empty `[models]` fields mean the active lane. `mock` shares one `MockProvider` for all roles.
3. **Scheduler** reuses `ResourceManager::acquire_agent` (`subagent.max_parallel` / `resources.max_parallel_agents`).
4. **Activation (both):**
   - Deterministic: `review_on_finish` / `debug_on_verify_fail` (default **false** so existing mock e2e stays unchanged).
   - LLM-driven: additive `AgentDecision::Delegate { role, objective, paths }` (old JSON still parses).
5. **Role tool allowlists** filter schemas before policy; every execution still goes through `ToolRegistry` → `PolicyEngine`. Subagents cannot pause the parent for approval (`ApprovalRequired` → tool failure message).
6. **Minimal context only** — brief paths via `safe_path` + optional `extra_context`; never the parent message dump.
7. Emit existing `agent.spawned` / `agent.completed` events. Nested `delegate` from a subagent is rejected.

## Alternatives
- New `raya-subagent` crate immediately — extra workspace churn before MCP needs it.
- Phase/TaskPhase expansion for Reviewing — unnecessary; existing Verifying→Fixing covers rework.
- Always-on reviewer — would break `MockProvider::default_script` length; defaults stay off.

## Consequences
- Config expands: `[models]`, extended `[subagent]` fields; CLI gains `raya agent llm routes`.
- `AppState` holds `ModelRouter` instead of a bare `LlmProvider`.
- Known limitations remain: in-memory `pending_approval` across process boundaries; `add_tokens` hard-ceiling errors still ignored with `let _ =` (pre-existing).
- Follow-ups: Memory (migration 0003), MCP, DAG, durable approval resume.
