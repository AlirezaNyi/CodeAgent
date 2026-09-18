# ADR 0013: Optional execution DAG

## Status
Accepted

## Context
RFC §13 describes an execution DAG for complex tasks. Slice A delivered ephemeral subagents and a model router; Slice B delivered durable approval and memory. Plans still ran as a sequential parent LLM loop (`steps` + `tool_call`). A full DAG state machine or new crate would break `MockProvider::default_script` and expand `TaskPhase` / `EventKind` contracts.

## Decision
1. **Optional overlay on `ExecutionPlan`.** Additive `nodes: Vec<DagNode>` with `#[serde(default)]`. Empty/absent nodes keep today’s sequential loop unchanged.
2. **No new `AgentDecision`, `TaskPhase`, or `EventKind`.** DAG workers reuse `agent.spawned` / `agent.completed` with payload `dag_node_id`.
3. **Wave scheduler** in `raya-agent::subagent::dag` over existing `SubagentRunner`, bounded by `subagent.max_parallel` (semaphore) and `max_dag_nodes` (default 8). Invalid graphs (`validate_dag`) are skipped with a warning — non-fatal.
4. **Persist for observability** via migration `0004` `task_dag_nodes` (status pending|running|completed|failed|skipped). CLI `raya agent dag` and HTTP `GET /v1/tasks/{id}/dag`. Rows deleted on terminal phases.
5. **Skip-on-fail.** A failed node skips dependents; the parent task is not auto-failed — the parent LLM receives a DAG report and may `needs_fix` / `finish`.
6. **Crash-resume of in-flight waves (Slice E).** Before DAG waves, persist a checkpoint (`pending_call = None`). `Orchestrator::run` may resume a non-terminal task with incomplete `task_dag_nodes`: reset `running` → `pending`, do **not** `replace_dag`, continue from `ready_nodes`. No new `TaskPhase` / `EventKind` / migration. CLI and MCP `resume` reuse the same path (HTTP approve resumes after grant; there is no separate HTTP `/resume` route).

## Alternatives
- New `TaskPhase::RunningDag` + EventKinds — contract churn without product need.
- Separate `raya-subagent` crate — deferred (ADR 0011); MCP did not require the boundary (ADR 0014).
- Replay DAG from events only — weaker CLI/HTTP status than a small status table.

## Consequences
- Prompts may emit `plan.nodes`; operators inspect status via CLI/HTTP; crashed mid-DAG tasks resume via `raya agent resume`.
- Phase 4 leftovers: richer project memory curation, Cursor extension, Streamable HTTP MCP, Web UI.
