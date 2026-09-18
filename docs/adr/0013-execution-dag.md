# ADR 0013: Optional execution DAG

## Status
Accepted

## Context
RFC §13 describes an execution DAG for complex tasks. Slice A delivered ephemeral subagents and a model router; Slice B delivered durable approval and memory. Plans still ran as a sequential parent LLM loop (`steps` + `tool_call`). A full DAG state machine or new crate would break `MockProvider::default_script` and expand `TaskPhase` / `EventKind` contracts.

## Decision
1. **Optional overlay on `ExecutionPlan`.** Additive `nodes: Vec<DagNode>` with `#[serde(default)]`. Empty/absent nodes keep today’s sequential loop unchanged.
2. **No new `AgentDecision`, `TaskPhase`, or `EventKind`.** DAG workers reuse `agent.spawned` / `agent.completed` with payload `dag_node_id`.
3. **Wave scheduler** in `raya-agent::subagent::dag` over existing `SubagentRunner`, bounded by `subagent.max_parallel` (semaphore) and `max_dag_nodes` (default 8). Invalid graphs (`validate_dag`) are skipped with a warning — non-fatal.
4. **Persist for observability** via migration `0004` `task_dag_nodes` (status pending|running|completed|failed|skipped). CLI `raya agent dag` and HTTP `GET /v1/tasks/{id}/dag`. Rows deleted on terminal phases. **No crash-resume** of an in-flight DAG in this slice (`Orchestrator::run` still only starts from `Created` or `WaitingApproval`).
5. **Skip-on-fail.** A failed node skips dependents; the parent task is not auto-failed — the parent LLM receives a DAG report and may `needs_fix` / `finish`.

## Alternatives
- New `TaskPhase::RunningDag` + EventKinds — contract churn without product need.
- Separate `raya-subagent` crate — deferred until MCP needs a boundary (ADR 0011).
- Replay DAG from events only — weaker CLI/HTTP status than a small status table.

## Consequences
- Prompts may emit `plan.nodes`; operators inspect status via CLI/HTTP.
- Follow-up: crash-resume of DAG waves, richer project memory curation. MCP landed in ADR 0014.
