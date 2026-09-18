# RAYA Agent — Product Requirements Document (PRD)

## 1. Document Control

- Product: RAYA Agent
- Document: PRD
- Version: 1.0
- Status: Draft for implementation
- Target platform: macOS / Linux initially
- Primary language: Rust
- Primary UI integration: Cursor via CLI / MCP / HTTP
- Deployment model: Local-first, single process, single binary

## 2. Product Vision

RAYA Agent is a local, resource-efficient agent runtime for software development.

It is designed to separate the agent execution/runtime layer from the editor UI. Cursor remains the primary development interface, while RAYA Agent owns orchestration, context construction, tools, policies, subagents, memory, observability, and resource management.

The central product objective is:

> Perform useful software-engineering tasks with bounded context, bounded memory, bounded concurrency, and explicit execution policies.

## 3. Problem

Current coding-agent workflows can consume substantial RAM and CPU because they may:

- repeatedly scan large repositories;
- send excessive repository context to models;
- spawn multiple processes or agents without strict limits;
- duplicate context between subagents;
- keep large histories in memory;
- execute tools without centralized resource controls;
- lack a persistent, incremental repository index.

RAYA Agent addresses these problems through:

- incremental indexing;
- relevance-ranked context retrieval;
- strict token budgets;
- bounded subagent concurrency;
- centralized tool execution;
- explicit approval policies;
- persistent local state in SQLite;
- cancellation and resource controls.

## 4. Goals

### G1 — Local-first execution
The core runtime must operate locally without requiring a cloud control plane.

### G2 — Low resource usage
The runtime must avoid unnecessary memory consumption and unbounded concurrency.

### G3 — High-quality repository context
The agent should retrieve only the files, symbols, dependencies, Git history, and task information relevant to the current task.

### G4 — Controlled execution
All filesystem, shell, Git, network, and potentially destructive operations must pass through a policy-aware tool runtime.

### G5 — Extensibility
LLM providers, tools, subagents, context sources, and integrations must be replaceable through stable Rust interfaces.

### G6 — Observable execution
Every important agent, model, tool, and task transition should be observable through structured events.

### G7 — Cursor compatibility
The runtime should be usable from Cursor without requiring Cursor itself to be modified.

## 5. Non-Goals

The initial product will NOT:

- replace Cursor;
- implement a custom LLM;
- require Kubernetes;
- require Redis/PostgreSQL/Kafka;
- use a vector database by default;
- implement browser automation;
- run distributed agents;
- maintain dozens of permanent agents;
- become a general-purpose autonomous computer-control platform.

## 6. Target Users

Primary:

- software developers;
- technical leads;
- DevOps engineers;
- engineering teams using Cursor;
- organizations requiring local execution and policy control.

Secondary:

- platform engineering teams;
- internal developer platform teams;
- teams building custom coding-agent workflows.

## 7. Core User Journeys

### Journey A — Run a coding task

1. Developer opens a repository.
2. Developer invokes RAYA Agent with a task.
3. Agent analyzes the request.
4. Context Engine searches the repository.
5. Relevant files/symbols/dependencies are selected.
6. Planner creates a structured execution plan.
7. Agent executes tools under policy.
8. Tests/builds are executed.
9. Reviewer verifies changes.
10. Agent fixes failures when permitted.
11. Task completes with an event history and Git diff.

### Journey B — Explain code

1. Developer requests an explanation of a file/function.
2. Context Engine resolves the relevant symbol and dependencies.
3. Agent retrieves bounded context.
4. LLM generates an explanation.
5. No repository-wide context is sent unnecessarily.

### Journey C — Debug failing tests

1. Developer provides failing test/error.
2. Agent searches code and Git history.
3. Agent identifies likely affected symbols.
4. Debugger subagent may be spawned if enabled.
5. Agent modifies code.
6. Tests are rerun.
7. Agent verifies the fix.

### Journey D — Protected operation

1. Agent requests a sensitive operation.
2. Policy Engine evaluates it.
3. If approval is required, execution pauses.
4. User approves/rejects.
5. Tool executes only after approval.

## 8. Product Architecture

```text
Cursor
  │
  ├── CLI
  ├── MCP
  └── HTTP
       │
       ▼
┌───────────────────────────────┐
│        RAYA Agent Rust        │
│                               │
│  Orchestrator                 │
│  Planner / Scheduler          │
│  Context Engine               │
│  Tool Runtime                 │
│  Policy Engine                │
│  Resource Manager             │
│  Memory                       │
│  Event Store                  │
│  LLM Gateway                  │
│  Subagent Runtime             │
│  Repository Index             │
└───────────────┬───────────────┘
                │
       ┌────────┼─────────┐
       ▼        ▼         ▼
    SQLite    Git/FS    LLM APIs
```

## 9. Functional Requirements

### FR-001 Task lifecycle
The system shall create, execute, monitor, cancel, and complete agent tasks.

### FR-002 Structured planning
The planner shall produce structured plans rather than relying on unstructured natural-language instructions.

### FR-003 Context retrieval
The system shall retrieve relevant repository context within a configured token budget.

### FR-004 Repository search
The system shall support text, file, symbol, and dependency-oriented search.

### FR-005 File operations
The system shall provide controlled read/write/patch operations.

### FR-006 Shell execution
The system shall execute shell commands through a policy-aware executor.

### FR-007 Git operations
The system shall inspect status/diff/log and optionally create commits.

### FR-008 Verification
The system shall support test and build execution.

### FR-009 Cancellation
Running tasks and child processes shall be cancellable.

### FR-010 Events
Important state changes and tool executions shall produce structured events.

### FR-011 Subagents
The system shall support bounded, ephemeral subagents.

### FR-012 Model routing
Different task phases shall be able to use different LLM providers/models.

### FR-013 Memory
The system shall persist useful task/project/decision memory locally.

### FR-014 Policy
Sensitive operations shall require configurable approval.

### FR-015 Resource limits
The system shall enforce configurable limits for memory, processes, tools, agents, iterations, and tokens.

## 10. Non-Functional Requirements

### NFR-001 Performance
Startup target: < 200 ms for the local CLI path under normal conditions.

### NFR-002 Memory
Target operating envelope:

- idle: < 100 MB;
- normal task: < 300 MB;
- heavy task: < 600 MB;
- indexing: < 500 MB where practical.

These are engineering targets, not hard compatibility guarantees.

### NFR-003 Context bound
No agent request may exceed its configured context budget.

### NFR-004 Concurrency
No unbounded agent/tool/process concurrency is permitted.

### NFR-005 Reliability
Cancellation, failure, and process termination must leave persistent task state consistent.

### NFR-006 Security
Secrets must not be written to normal logs.

### NFR-007 Portability
The core must work on macOS and Linux.

### NFR-008 Maintainability
The codebase must use explicit module boundaries and typed interfaces.

## 11. MVP

MVP includes:

- Rust workspace;
- local CLI;
- Agent Loop;
- LLM Gateway abstraction;
- OpenAI-compatible provider;
- mock provider;
- local LLM lanes (named OpenAI-compatible backends; ADR 0010);
- phase/role model router over lanes (ADR 0011);
- bounded ephemeral subagents (Planner/Coder/Reviewer/Debugger; ADR 0011);
- filesystem tools;
- search tool using ripgrep;
- shell tool;
- Git tool;
- SQLite event/task store;
- context token budget;
- policy engine;
- cancellation;
- structured logs;
- basic HTTP API.

MVP excludes:

- full Tree-sitter graph;
- advanced subagent DAG / durable memory / MCP (later Phase 3);
- vector search;
- Cursor extension;
- advanced memory retrieval;
- distributed execution.

## 12. Roadmap

### Phase 1 — Vertical Slice
CLI → Task → Context → LLM → Tools → Verify → Events.

### Phase 2 — Repository Intelligence
Tree-sitter, incremental index, symbols, dependencies, FTS5, Git intelligence.

### Phase 3 — Agentic Runtime
Subagents, scheduler, DAG, model router, memory, MCP.

### Phase 4 — Developer Experience
Cursor integration, Web UI, advanced policy UX, resource dashboard.

## 13. Success Metrics

- repository context tokens reduced compared with naive whole-repository prompting;
- bounded peak memory during normal agent tasks;
- successful cancellation of long-running tasks;
- reproducible task event history;
- successful execution of common coding tasks;
- incremental indexing avoids full repository rescans after small changes.

## 14. Definition of Done

A feature is complete when:

1. implementation is tested;
2. relevant documentation is updated;
3. formatting passes;
4. Clippy passes for applicable targets;
5. unit/integration tests pass;
6. resource limits are respected;
7. security implications are considered;
8. an atomic Git commit is created according to repository conventions.
