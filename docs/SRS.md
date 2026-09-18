# RAYA Agent — Software Requirements Specification (SRS)

## 1. Scope

This SRS defines the software behavior, interfaces, constraints, and acceptance criteria for RAYA Agent.

## 2. Actors

- Developer
- Agent Orchestrator
- LLM Provider
- Tool Runtime
- Policy Engine
- Repository Indexer
- Subagent
- Local API Client
- Cursor/CLI client

## 3. System Requirements

### SYS-001
The application shall run as a local Rust process.

### SYS-002
The application shall persist task state in SQLite.

### SYS-003
The application shall support CLI and local HTTP interfaces.

### SYS-004
The application shall expose health status.

## 4. Task Requirements

### REQ-TASK-001
The system shall accept a task request containing project and user request.

Acceptance:
- valid request creates a unique task ID;
- invalid project/request returns structured validation error.

### REQ-TASK-002
The system shall expose task state.

Acceptance:
- caller can retrieve phase, status, timestamps, and error state.

### REQ-TASK-003
The system shall support cancellation.

Acceptance:
- cancellation transitions task to `Cancelled`;
- active child operations receive cancellation.

## 5. Planning Requirements

### REQ-PLAN-001
The planner shall return a typed execution plan.

### REQ-PLAN-002
Plans shall contain ordered steps.

### REQ-PLAN-003
Plans shall declare expected tools and verification strategy where possible.

### REQ-PLAN-004
Malformed model output shall not be executed as a tool command.

## 6. Context Requirements

### REQ-CONTEXT-001
The context engine shall search repository content.

### REQ-CONTEXT-002
The context engine shall respect `max_tokens`.

### REQ-CONTEXT-003
The context engine shall rank retrieved material.

### REQ-CONTEXT-004
The engine shall avoid sending duplicate files/chunks.

### REQ-CONTEXT-005
Context assembly shall be deterministic for identical repository/task state where practical.

## 7. Index Requirements

### REQ-INDEX-001
The indexer shall detect file changes.

### REQ-INDEX-002
Unchanged files shall not be reparsed unnecessarily.

### REQ-INDEX-003
Supported syntax trees shall be parsed with Tree-sitter where implemented.

### REQ-INDEX-004
Symbols shall include enough location metadata to reopen their source.

### REQ-INDEX-005
Dependency edges shall be persisted.

## 8. Tool Requirements

### REQ-TOOL-001
Tools shall implement a common typed interface.

### REQ-TOOL-002
Every tool call shall have a task ID.

### REQ-TOOL-003
Every tool call shall be policy-checked.

### REQ-TOOL-004
Tool results shall have bounded output size.

### REQ-TOOL-005
Tool execution shall support cancellation where technically possible.

## 9. Filesystem Requirements

### REQ-FS-001
Read operations shall support project-relative paths.

### REQ-FS-002
Write operations shall be policy-controlled.

### REQ-FS-003
Path traversal outside allowed roots shall be rejected by default.

### REQ-FS-004
Patch operations shall report modified files.

## 10. Shell Requirements

### REQ-SHELL-001
Shell commands shall have configurable timeout.

### REQ-SHELL-002
Shell commands shall have stdout/stderr limits.

### REQ-SHELL-003
Shell processes shall be cancellable.

### REQ-SHELL-004
Shell execution shall obey policy.

### REQ-SHELL-005
Exit code, stdout, stderr, duration, and cancellation status shall be recorded.

## 11. Git Requirements

### REQ-GIT-001
The system shall read Git status.

### REQ-GIT-002
The system shall generate diffs.

### REQ-GIT-003
The system shall inspect recent history.

### REQ-GIT-004
Commit creation shall be separately policy-controlled.

## 12. LLM Requirements

### REQ-LLM-001
The LLM layer shall use a provider abstraction.

### REQ-LLM-002
Provider errors shall be normalized.

### REQ-LLM-003
Requests shall include task/context metadata required for tracing but never secrets unnecessarily.

### REQ-LLM-004
Structured outputs shall be validated before use.

### REQ-LLM-005
Token usage shall be recorded where provider data is available.

## 13. Subagent Requirements

### REQ-AGENT-001
Subagents shall have explicit roles.

### REQ-AGENT-002
Subagents shall have bounded token and time budgets.

### REQ-AGENT-003
Maximum concurrent subagents shall be configurable.

### REQ-AGENT-004
Subagents shall receive only required context.

### REQ-AGENT-005
Subagents shall inherit cancellation.

## 14. Policy Requirements

### REQ-POLICY-001
Policy rules shall classify tool operations by risk.

### REQ-POLICY-002
Approval-required operations shall pause execution.

### REQ-POLICY-003
A denied operation shall not execute.

### REQ-POLICY-004
Policy decisions shall be logged as events without leaking secrets.

## 15. Resource Requirements

### REQ-RESOURCE-001
Maximum parallel tools shall be configurable.

### REQ-RESOURCE-002
Maximum parallel agents shall be configurable.

### REQ-RESOURCE-003
Maximum process count shall be configurable.

### REQ-RESOURCE-004
Task token budgets shall be enforced.

### REQ-RESOURCE-005
Maximum iterations/tool calls shall be enforced.

## 16. Event Requirements

### REQ-EVENT-001
Task lifecycle changes shall emit events.

### REQ-EVENT-002
Tool execution shall emit start/completion events.

### REQ-EVENT-003
LLM requests/responses shall emit trace events without exposing secrets.

### REQ-EVENT-004
Events shall include timestamp, task ID, event type, and structured payload.

## 17. HTTP Requirements

### REQ-HTTP-001
API shall bind to localhost by default.

### REQ-HTTP-002
POST `/v1/tasks` shall create a task.

### REQ-HTTP-003
GET `/v1/tasks/:id` shall return task state.

### REQ-HTTP-004
POST `/v1/tasks/:id/cancel` shall cancel a task.

### REQ-HTTP-005
GET `/v1/tasks/:id/events` shall return task events.

### REQ-HTTP-006
GET `/health` shall return process health.

### REQ-HTTP-007
GET `/metrics` shall expose operational metrics where implemented.

## 18. CLI Requirements

### REQ-CLI-001
`raya agent run` shall submit a task.

### REQ-CLI-002
`raya agent status` shall report task status.

### REQ-CLI-003
`raya agent logs` shall display structured task events.

### REQ-CLI-004
`raya agent cancel` shall cancel a task.

### REQ-CLI-005
`raya agent index` shall trigger indexing.

## 19. Configuration Requirements

### REQ-CONFIG-001
Project-specific configuration shall be supported through `.raya/config.toml`.

### REQ-CONFIG-002
Defaults shall be safe and resource-bounded.

### REQ-CONFIG-003
Invalid configuration shall fail with actionable errors.

## 20. Observability

Required logging fields where applicable:

```text
timestamp
level
task_id
project_id
agent_id
tool
event
duration_ms
result
error
```

Secrets must be redacted.

## 21. Testing

Required layers:

### Unit
- policy;
- token budgeting;
- ranking;
- task state transitions;
- configuration;
- tool validation.

### Integration
- SQLite persistence;
- CLI;
- HTTP API;
- filesystem tool;
- shell executor;
- Git tool;
- mock LLM.

### End-to-end
At least one complete:

```text
CLI → task → mock LLM → tool → test → completed
```

## 22. Performance Acceptance

The implementation should demonstrate:

- bounded context size;
- bounded number of concurrent tools;
- bounded number of subagents;
- cancellation of a long-running shell command;
- incremental indexing of changed files;
- no obvious memory growth across repeated completed tasks.

Performance thresholds should be benchmarked and refined after a baseline exists.

## 23. Security Acceptance

The implementation must demonstrate:

- path traversal rejection;
- command policy enforcement;
- secret redaction;
- localhost-only default binding;
- destructive operation approval;
- bounded process execution.

## 24. MVP Acceptance Scenario

Given a local repository and a request:

> Add validation to the login endpoint and update tests.

The system must be able to:

1. create a task;
2. inspect repository state;
3. retrieve relevant context;
4. generate a structured plan;
5. modify files through tools;
6. run tests;
7. detect failure;
8. attempt a bounded fix if configured;
9. finish with a structured task state;
10. expose the event history.

## 25. Deliverables

The initial implementation shall produce:

```text
docs/PRD.md
docs/RFC.md
docs/SRS.md
docs/adr/
.raya/config.toml.example
.raya/rules/
README.md
Cargo.toml
crates/*
migrations/*
tests/*
```
