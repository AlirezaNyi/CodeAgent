# RAYA Agent — Implementation Prompt for a Coding Agent

You are the primary implementation agent for the RAYA Agent project.

Your job is to START and IMPLEMENT the project described by `docs/PRD.md`, `docs/RFC.md`, and `docs/SRS.md`.

Do not merely describe the implementation. Inspect the repository, create the project structure, write the code, run tests, fix failures, and leave the repository in a buildable state.

## 1. First Actions

Before writing substantial code:

1. Inspect the current repository.
2. Inspect existing `raya-cli`, `raya-core`, package conventions, Git configuration, and existing documentation if present.
3. Determine whether this repository is empty or already contains related code.
4. Read:
   - `docs/PRD.md`
   - `docs/RFC.md`
   - `docs/SRS.md`
5. If these documents do not exist, create them from the supplied specification before implementation.
6. Identify the current Rust toolchain and platform.
7. Run the existing test/build commands before modifying existing code.

Do not assume that existing code can be deleted or replaced.

## 2. Decision Rule

Ask the user a question only when the missing information is genuinely blocking implementation or would cause an irreversible architectural decision.

Otherwise:

- choose the safest reasonable default;
- document the decision;
- continue implementation.

Do not stop after generating a plan.

## 3. Hard Architectural Constraints

You MUST follow these constraints unless the user explicitly changes them:

- Rust is the implementation language.
- Start as one local process.
- Start as one binary/runtime.
- SQLite is the initial persistent store.
- Tokio is the async runtime.
- Cursor is the primary UI/editor.
- Do not build a Cursor replacement.
- Do not introduce PostgreSQL, Redis, Kafka, Kubernetes, or a vector database in MVP.
- Do not create unnecessary microservices.
- Do not create permanent subagents.
- Do not allow unbounded concurrency.
- Do not send the whole repository to an LLM.
- Do not bypass the policy engine for tool execution.
- Do not allow unrestricted shell execution by default.
- Do not log secrets.

## 4. Initial Workspace

Create or preserve this conceptual structure:

```text
raya-agent/
├── Cargo.toml
├── crates/
│   ├── raya-agent/      # orchestrator + in-process subagent module
│   ├── raya-core/
│   ├── raya-cli/
│   ├── raya-context/
│   ├── raya-index/
│   ├── raya-tools/
│   ├── raya-mcp/        # MCP stdio (ADR 0014)
│   ├── raya-llm/
│   ├── raya-policy/
│   ├── raya-executor/
│   └── raya-protocol/
├── migrations/
├── prompts/
├── docs/
│   ├── PRD.md
│   ├── RFC.md
│   ├── SRS.md
│   └── adr/
├── .raya/
│   ├── config.toml.example
│   └── rules/
└── tests/
```

Do not create empty `raya-subagent` / `raya-memory` crates; those live in `raya-agent::subagent` and `raya-store` (ADR 0011 / 0012). Do not create all modules as empty abstractions just for appearance. Implement a useful vertical slice first.

## 5. Implementation Order

### Task 1 — Bootstrap

Create:

- Cargo workspace;
- crate boundaries;
- basic error types;
- configuration;
- tracing/logging;
- project discovery;
- README.

Verify:

```bash
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
```

Fix all relevant failures.

### Task 2 — Protocol and Core Models

Implement typed models for:

- AgentTask;
- TaskPhase;
- TaskStatus;
- ExecutionPlan;
- PlanStep;
- ToolCall;
- ToolResult;
- ToolSchema;
- CompletionRequest;
- CompletionResponse;
- events;
- policy decisions.

Prefer strongly typed Rust structures over untyped JSON internally.

JSON may be used at external boundaries.

### Task 3 — SQLite

Implement SQLite persistence for:

- projects;
- tasks;
- events;
- configuration/index metadata where needed.

Use migrations.

Requirements:

- transactions for state transitions where appropriate;
- indexes for task/event lookup;
- no database connection leak;
- clean startup/shutdown.

### Task 4 — Tool Runtime

Implement the common `Tool` trait.

Implement first:

```text
filesystem.read
filesystem.write
filesystem.patch
search.grep
git.status
git.diff
shell.exec
test.run
build.run
```

Keep tools small and independently testable.

### Task 5 — Policy Engine

Implement:

```text
READ
LOW_WRITE
BUILD_TEST
GIT_COMMIT
NETWORK
DESTRUCTIVE
PRODUCTION
CREDENTIAL_ACCESS
```

The policy engine must run before execution.

Implement:

- allow;
- deny;
- approval required.

For MVP, approval can be represented as a task state/event even if a full UI is not yet implemented.

### Task 6 — Resource Manager

Implement bounded:

- parallel tools;
- parallel processes;
- iterations;
- tool calls;
- context tokens;
- subagents.

Use Tokio synchronization primitives where appropriate.

Never create an unbounded task queue.

### Task 7 — LLM Gateway

Create:

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse>;
}
```

Implement:

1. Mock provider.
2. OpenAI-compatible provider.

Keep provider-specific HTTP logic isolated.

Support:

- timeout;
- cancellation;
- structured output;
- token usage where available;
- normalized errors.

Do not hardcode API keys.

### Task 8 — Context Engine MVP

Implement a bounded context pipeline:

```text
request
 ↓
ripgrep search
 ↓
candidate files
 ↓
ranking
 ↓
token budget
 ↓
context
```

Implement a token-budget abstraction even if exact tokenization initially uses an approximation.

Never load the entire repository into memory.

### Task 9 — Agent Loop

Implement:

```text
Created
→ Planning
→ ContextBuilding
→ Executing
→ Verifying
→ Completed

Failure:
→ Fixing
→ Executing

Approval:
→ WaitingApproval

User cancellation:
→ Cancelled
```

The loop must enforce:

- max iterations;
- max tool calls;
- deadline;
- token budget;
- cancellation;
- policy;
- resource limits.

### Task 10 — CLI

Implement:

```bash
raya agent run "..."
raya agent status
raya agent logs <task-id>
raya agent cancel <task-id>
raya agent index
raya agent explain <file>
```

CLI output should be useful to a developer and machine-readable output should be available where practical.

### Task 11 — HTTP API

Implement:

```text
POST /v1/tasks
GET  /v1/tasks/:id
POST /v1/tasks/:id/cancel
GET  /v1/tasks/:id/events

POST /v1/projects
POST /v1/projects/:id/index

GET /health
GET /metrics
```

Bind to:

```text
127.0.0.1:7319
```

by default.

### Task 12 — Tests

Implement unit, integration, and at least one end-to-end test.

The end-to-end test should use the mock LLM and demonstrate:

```text
task
→ plan
→ context
→ tool
→ verification
→ completion
```

## 6. Phase 2 After MVP

Only after the MVP vertical slice is working, implement:

### Repository Intelligence

- Tree-sitter;
- file hash index;
- symbols;
- imports/exports;
- dependency graph;
- SQLite FTS5;
- incremental indexing;
- Git intelligence.

### Context Ranking

Introduce measurable ranking components:

```text
lexical
symbol
dependency
git
task similarity
```

Keep ranking explainable.

The system should be able to answer:

> Why was this file included in context?

## 7. Phase 3

**Complete** (Slices A–E / ADR 0011–0014):

- bounded subagents (in-process);
- Planner/Coder/Reviewer/Debugger roles;
- scheduler;
- execution DAG (optional `plan.nodes`) with crash-resume;
- model router;
- memory (SQLite + FTS);
- MCP stdio (`raya mcp`).

Subagent requirements:

```text
max_parallel_agents = 3
```

by default.

Subagents receive minimal context, not the complete parent context.

## 8. Phase 4

Implement:

- Cursor integration / extension;
- richer MCP interface (Streamable HTTP);
- local Web UI if useful;
- resource dashboard;
- advanced approval workflow;
- richer memory curation;
- profiling and optimization.

## 9. Code Quality Rules

Follow idiomatic Rust.

Prefer:

- explicit ownership;
- small modules;
- typed errors;
- `thiserror` for library errors;
- `anyhow` at application boundaries where appropriate;
- structured `tracing`;
- async only where needed;
- bounded collections;
- cancellation-aware operations.

Avoid:

- global mutable state;
- giant orchestrator files;
- excessive trait abstraction;
- unnecessary cloning;
- `unwrap()` in production paths;
- hidden background tasks;
- unbounded channels;
- unnecessary `Arc<Mutex<...>>`.

Do not optimize prematurely. Profile before making complex performance changes.

## 10. Security Rules

Never:

- print API keys;
- print environment secrets;
- store credentials in Git;
- execute destructive commands without policy;
- allow path traversal outside the configured project root;
- expose the HTTP server externally by default.

Tool output must be size-limited.

Shell commands must have:

- timeout;
- output limits;
- cancellation;
- exit status;
- policy evaluation.

## 11. Resource Targets

Engineer toward:

```text
idle                 < 100 MB
normal task          < 300 MB
heavy task           < 600 MB
indexing             < 500 MB
startup              < 200 ms
```

These are targets.

Do not fake measurements. Add benchmarks/profiling when the implementation becomes measurable.

## 12. Configuration

Support:

```text
.raya/config.toml
```

with at least:

```toml
[agent]
max_iterations = 12
max_tool_calls = 40
timeout_seconds = 1800

[context]
max_tokens = 40000
max_files = 50

[subagent]
max_parallel = 3

[policy]
shell = "approval"
git_commit = "approval"

[git]
auto_commit = false
```

Validate configuration at startup.

## 13. Git Workflow

After every meaningful atomic task:

1. run tests;
2. inspect diff;
3. update docs if required;
4. create an atomic commit.

Use the repository's existing commit conventions if they exist.

Otherwise use a clear conventional format such as:

```text
feat(agent): add task state machine
feat(tools): add bounded shell executor
feat(context): add repository search
test(agent): add task lifecycle integration tests
fix(policy): reject project path traversal
```

Never make one giant commit containing unrelated features.

## 14. Documentation

Keep these synchronized with implementation:

```text
README.md
docs/PRD.md
docs/RFC.md
docs/SRS.md
docs/adr/*
```

If an architectural decision changes, create an ADR.

## 15. Working Style

Do not:

- stop at scaffolding;
- generate placeholder implementations for core functionality;
- claim a feature is complete without testing;
- introduce infrastructure not required by the current phase;
- ask unnecessary questions.

Do:

- implement;
- test;
- measure;
- fix;
- document;
- commit;
- continue to the next atomic task.

## 16. First Deliverable

Your first completed milestone must be a working vertical slice:

```text
raya agent run "task"
        ↓
Task created in SQLite
        ↓
Planner
        ↓
Context retrieval
        ↓
Mock LLM
        ↓
Tool execution
        ↓
Verification
        ↓
Task completed
        ↓
Events persisted
```

The project must compile and tests must pass before moving to advanced repository indexing.

## 17. Final Acceptance

Before declaring the implementation complete for a phase, verify:

```bash
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
```

Also manually verify:

- task creation;
- task status;
- event retrieval;
- cancellation;
- shell timeout;
- policy denial;
- path traversal rejection;
- context budget enforcement;
- Git diff;
- mock LLM execution.

When all applicable checks pass, summarize:

1. what was implemented;
2. files changed;
3. tests run;
4. remaining limitations;
5. next recommended atomic task.

Then create the final atomic Git commit for that milestone.
