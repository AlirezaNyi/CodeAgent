# ADR 0012: Durable approval resume and memory

## Status
Accepted

## Context
ADR 0011 left approval state in-process: `pending_approval` and message history lived only in the orchestrator. `raya agent approve` updated SQLite but no process resumed the tool call. `Store::is_approved` collapsed pending and denied into `false`, so deny was indistinguishable. RFC §15 Memory tables did not exist yet.

## Decision
1. **Task checkpoints (migration 0003).** Persist `task_checkpoints(task_id, pending_call_json, messages_json, review_rounds, updated_at)` before returning `WaitingApproval`. Cap messages to the last 40 (redacted). On a new `Orchestrator::run` when phase is `WaitingApproval`, restore from checkpoint (emit `phase.changed` with `{"resumed": true}`); missing checkpoint fails the task.
2. **Approval status tri-state.** `Store::approval_status(call_id) -> Option<bool>`: `None` pending, `Some(true)` granted, `Some(false)` denied. Deny emits `approval.denied`, appends a user-visible denial message, clears pending, deletes checkpoint, and continues the loop.
3. **Resume execution path.** Granted resume uses `ToolRegistry::execute_approved` so policy does not re-request approval for the already-granted call. Checkpoint is deleted after the approved tool runs and on terminal phases.
4. **CLI / HTTP auto-resume.** `raya agent approve` defaults to set_approval then `Orchestrator::run` (`--deny`, `--no-resume` optional). `raya agent resume <task-id>` resumes only. HTTP `POST /v1/tasks/{id}/approve` with `{call_id, granted?}` sets approval and spawns resume like `create_task`.
5. **Memory (same migration).** Tables `memories` + `memories_fts` with kinds `task|project|decision|agent`. Orchestrator writes Task memory on `Completed` and Decision memory on plan creation (gated by `[memory] enabled`). ContextBuilding injects a bounded `search_memories` block (`max_items` / `max_tokens`) and adds `memories: [{id, kind}]` to `context.retrieved`. Never dump all memory.
6. **Surfaces.** CLI `raya agent memory list|search|add|forget`; HTTP `GET /v1/projects/{id}/memory?q=&kind=&limit=`; `[memory]` in config template.

## Alternatives
- Replay messages from the event log — heavier, lossy for LLM turns, and couples resume to event schema churn.
- Keep approval in-process only — fails CLI/HTTP approve across process boundaries (the known ADR 0011 defect).
- Separate migration for memory vs checkpoints — unnecessary; both are Phase 3 Slice B persistence.

## Consequences
- Store API grows checkpoint and memory helpers; `is_approved` remains for callers that only need “granted?”.
- Subagents still cannot pause the parent for approval (ADR 0011).
- Follow-ups: MCP, execution DAG, richer agent/project memory curation.
