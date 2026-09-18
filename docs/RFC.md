# RAYA Agent — Request for Comments (RFC)

## 1. Status

- Version: 1.0
- Status: Accepted architecture baseline
- Scope: Runtime architecture and engineering decisions

## 2. Architectural Principles

1. Local-first.
2. Single process initially.
3. Single SQLite database.
4. Async runtime using Tokio.
5. Bounded resources.
6. Typed interfaces.
7. Context retrieval before LLM invocation.
8. Tools are policy-controlled.
9. Subagents are ephemeral.
10. Prefer simple local components before distributed infrastructure.

## 3. Workspace

```text
raya-agent/
├── Cargo.toml
├── crates/
│   ├── raya-agent/
│   ├── raya-core/
│   ├── raya-cli/
│   ├── raya-context/
│   ├── raya-index/
│   ├── raya-tools/
│   ├── raya-mcp/
│   ├── raya-llm/
│   ├── raya-subagent/
│   ├── raya-memory/
│   ├── raya-policy/
│   ├── raya-executor/
│   └── raya-protocol/
├── migrations/
├── prompts/
├── docs/
└── tests/
```

Dependency direction:

```text
raya-cli ───────────────┐
raya-mcp ───────────────┤
raya-protocol ──────────┤
                         ▼
                    raya-agent
                         │
        ┌────────────────┼─────────────────┐
        ▼                ▼                 ▼
   raya-context     raya-subagent     raya-executor
        │                │                 │
        ▼                ▼                 ▼
   raya-index        raya-llm         raya-policy
                         │
                         ▼
                      SQLite
```

Lower-level crates must not depend on the top-level orchestrator.

## 4. Runtime State Machine

```text
Created
  ↓
Planning
  ↓
ContextBuilding
  ↓
Executing
  ↓
Verifying
  ├── Completed
  └── Fixing
        ↓
     Executing

Any state:
  → WaitingApproval
  → Cancelled
  → Failed
```

## 5. Task Model

```rust
struct AgentTask {
    id: Uuid,
    project_id: Uuid,
    request: String,
    phase: TaskPhase,
    plan: Option<ExecutionPlan>,
    current_step: Option<String>,
    max_iterations: u32,
    max_tool_calls: u32,
    max_tokens: u64,
    context_token_budget: u64,
    deadline: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
```

## 6. Agent Loop

```text
request
  ↓
intent analysis
  ↓
plan
  ↓
context retrieval
  ↓
LLM decision
  ↓
tool call
  ↓
tool result
  ↓
LLM decision
  ↓
verification
  ↓
complete / fix
```

The loop must enforce:

- maximum iterations;
- maximum tool calls;
- token budget;
- deadline;
- cancellation token;
- resource limits;
- policy checks.

## 7. Context Engine

The Context Engine is the primary performance-critical component.

Pipeline:

```text
User Request
    ↓
Intent extraction
    ↓
Lexical search
    ↓
Symbol resolution
    ↓
Dependency expansion
    ↓
Git intelligence
    ↓
Relevance scoring
    ↓
Token budget allocation
    ↓
Context assembly
    ↓
LLM
```

Initial search mechanisms:

- ripgrep;
- SQLite FTS5;
- AST/symbol index;
- dependency graph;
- Git status/diff/log.

No vector database is required initially.

### Ranking

A conceptual score:

```text
score =
    lexical_score
  + symbol_score
  + dependency_score
  + git_score
  + task_similarity_score
```

The exact weights should be configurable after measurement.

The Context Engine must never silently exceed `MAX_CONTEXT_TOKENS`.

## 8. Repository Index

Use SQLite for:

- files;
- file hashes;
- symbols;
- imports/exports;
- dependencies;
- chunks;
- indexing metadata.

Tree-sitter should parse only changed files.

Incremental strategy:

```text
filesystem event
    ↓
hash file
    ↓
unchanged? ── yes → ignore
    │
    no
    ↓
parse changed file
    ↓
replace affected symbols/chunks/dependencies
```

## 9. Tool System

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> ToolSchema;

    async fn execute(
        &self,
        ctx: ToolContext,
        input: serde_json::Value,
    ) -> Result<ToolResult>;
}
```

Initial tools:

```text
filesystem.read
filesystem.write
filesystem.patch

search.grep
search.files
search.symbol

git.status
git.diff
git.log
git.commit

shell.exec
test.run
build.run

context.search
agent.spawn
```

Every tool execution receives a `ToolContext` containing task ID, project, policy, cancellation, and resource information.

## 10. Shell Executor

The shell executor must control:

- executable;
- arguments;
- working directory;
- environment;
- timeout;
- stdout limit;
- stderr limit;
- exit code;
- cancellation;
- process lifecycle.

Use process termination on cancellation/drop where supported.

Do not expose unrestricted shell execution by default.

## 11. Policy Engine

Risk classes:

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

Suggested defaults:

```text
read              → auto
write             → auto
test/build        → auto
package install   → approval
git commit        → configurable / approval
docker             → approval
network            → configurable
SSH                → restricted
production         → restricted
credentials       → restricted
destructive       → approval/restricted
```

The policy engine must evaluate both tool type and input.

## 12. Subagent Architecture

Subagents are workers, not permanent sessions.

Initial roles:

- Planner;
- Coder;
- Reviewer;
- Debugger.

Scheduler:

```rust
Semaphore(max_parallel_agents)
```

Each subagent gets:

- explicit task;
- minimal relevant context;
- restricted tools;
- token budget;
- timeout;
- cancellation token.

Do not duplicate the complete parent context.

**Slice A (ADR 0011):** implemented in-process under `raya-agent::subagent` with role allowlists, `ModelRouter` over named lanes, deterministic `review_on_finish` / `debug_on_verify_fail` (default off), and LLM `AgentDecision::Delegate`.

**Slice B (ADR 0012):** durable approval checkpoints + resume across processes; SQLite Memory (`task` / `project` / `decision` / `agent`) with bounded FTS recall.

**Slice C (ADR 0013):** optional `ExecutionPlan.nodes` DAG over in-process subagents (wave scheduler, `task_dag_nodes` status).

**Slice D (ADR 0014):** `raya-mcp` stdio MCP control plane for Cursor (`raya mcp`); tools wrap Store/Orchestrator only — no filesystem/shell MCP tools.

## 13. Execution DAG

For complex tasks:

```text
Task
 ↓
Analyze
 ├── Backend
 └── Frontend
      ↓
   Implement
      ↓
 ┌────┴────┐
Tests    Review
 └────┬────┘
      ↓
   Verify
      ↓
 Fix if needed
```

The DAG is optional for simple tasks.

**Slice C (ADR 0013):** plans may include additive `nodes: [{id, role, objective, paths, depends_on}]`. Empty/omitted nodes keep the sequential parent LLM loop. The orchestrator runs ready nodes in waves via existing subagents (`subagent.max_parallel`, `max_dag_nodes`). Status is persisted in `task_dag_nodes` for `raya agent dag` / `GET /v1/tasks/{id}/dag`. Failed nodes skip dependents; the parent task is not auto-failed. Crash-resume of an in-flight DAG is out of scope for this slice.

**Slice D (ADR 0014):** `crates/raya-mcp` is implemented (not a stub). Stdio MCP via `rmcp` exposes control-plane tools (`raya_run`, `raya_status`, `raya_logs`, `raya_approve`, `raya_resume`, `raya_cancel`, `raya_dag`, `raya_memory_*`). CLI `raya mcp` blocks on stdin for Cursor `mcp.json`. Streamable HTTP MCP and richer Cursor extension remain Phase 4.

## 14. LLM Gateway

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse>;
}
```

Providers:

- OpenAI-compatible HTTP (also used for local servers);
- Mock (offline);
- Named local **lanes** (see ADR 0010) for Ollama / LM Studio / proxies;
- Anthropic / Azure-native clients (future).

Use structured outputs wherever possible.

Model routing can later map:

```text
planning  → cheap/fast
coding    → strong
review    → medium
summary   → cheap
```

**Slice A:** `[models]` maps these roles to `[[llm.lanes]]` names via `raya-llm::ModelRouter` (ADR 0011). Empty values use the active lane.
## 15. Memory

SQLite-backed memory categories (migration `0003`, ADR 0012):

```text
task     — completed-task summary + request + modified files
project  — operator-curated project facts (`raya agent memory add`)
decision — plan summaries written on PlanCreated
agent    — reserved for future agent-scoped notes
```

Tables: `memories` + FTS5 `memories_fts`. Config `[memory] enabled / max_items / max_tokens`.

Retrieval: `Store::search_memories` (FTS, LIKE fallback) injected in ContextBuilding as a capped `## Project memory` block. Payload `context.retrieved.memories: [{id, kind}]`.

Never dump all historical memory into the prompt.

Durable approval (same migration): `task_checkpoints` stores redacted messages + pending tool call; CLI/HTTP approve resumes a new orchestrator process.

## 16. Event Store

Events include:

```text
task.created
task.started
plan.created
context.retrieved
llm.request
llm.response
tool.started
tool.completed
file.modified
test.started
test.failed
test.passed
agent.spawned
agent.completed
approval.requested
approval.granted
approval.denied
task.completed
task.failed
task.cancelled
```

Events must be structured and queryable.

## 17. Resource Manager

Token budget and RAM/process budget are separate concerns.

Example:

```toml
[resources]
max_memory_mb = 700
max_parallel_tools = 4
max_parallel_agents = 3
max_processes = 8
```

The system should expose resource usage through metrics/events where practical.

## 18. Cancellation

Use Tokio `CancellationToken`.

Cancellation must propagate:

```text
Task
 ├── LLM request
 ├── tool execution
 ├── shell process
 └── subagents
```

## 19. Local HTTP API

Bind only to localhost by default:

```text
127.0.0.1:7319
```

Endpoints:

```text
POST /v1/tasks
GET  /v1/tasks/:id
POST /v1/tasks/:id/cancel
GET  /v1/tasks/:id/events

POST /v1/projects
POST /v1/projects/:id/index

GET /health
GET /metrics

WS /ws/tasks/:id
```

Authentication is not required for the initial localhost-only MVP, but binding to non-loopback addresses must be explicitly configurable and protected.

## 20. CLI

```text
raya agent run "..."
raya agent status
raya agent logs <task-id>
raya agent cancel <task-id>
raya agent review
raya agent index
raya agent explain <file>
```

## 21. Project Configuration

`.raya/config.toml`

Example:

```toml
[project]
language = "auto"

[agent]
max_iterations = 12
max_tool_calls = 40
timeout_seconds = 1800

[context]
max_tokens = 40000
max_files = 50

[subagent]
max_parallel = 3

[models]
planning = "cheap"
coding = "strong"
review = "medium"
summary = "cheap"

[policy]
shell = "approval"
git_commit = "approval"

[git]
auto_commit = false

[models]
planning = "cheap"
coding = "strong"
review = "medium"
summary = "cheap"
```

Rules:

```text
.raya/rules/architecture.md
.raya/rules/coding.md
.raya/rules/testing.md
.raya/rules/git.md
```

## 22. Git Strategy

For task execution, optionally create:

```text
raya/task/<task-id>
```

Example commit format:

```text
fix(auth): handle expired refresh token
feat(agent): add bounded shell executor
refactor(context): add relevance ranking
```

The agent must inspect existing repository conventions before applying commit rules.

## 23. Security

Security requirements:

- never log API keys;
- redact credentials from tool output;
- restrict shell execution;
- prevent path traversal outside project root unless explicitly allowed;
- restrict network access according to policy;
- do not expose localhost API externally by default;
- prevent destructive operations without approval;
- apply output-size limits;
- apply process/time limits.

## 24. Technology Decisions

### Rust
Chosen for:

- predictable memory behavior;
- strong concurrency primitives;
- process management;
- low runtime overhead;
- reliable cancellation;
- single-binary distribution.

### SQLite
Chosen for:

- zero infrastructure;
- local persistence;
- FTS5;
- transactional state;
- simple deployment.

### Tree-sitter
Chosen for incremental syntax-aware indexing.

### ripgrep
Chosen for fast lexical repository search.

### Tokio
Chosen for asynchronous execution and cancellation.

## 25. Explicitly Rejected Initially

Do not introduce without measured need:

- PostgreSQL;
- Redis;
- Kafka;
- Kubernetes;
- vector DB;
- microservices;
- browser automation;
- distributed scheduler;
- custom model serving.

## 26. Failure Handling

Every failure should map to a structured state/event.

Examples:

```text
LLM timeout       → retry policy → failed/waiting
tool timeout      → terminate → failed
test failure      → Fixing
policy denial     → WaitingApproval
user cancellation → Cancelled
resource exceeded → Failed / Cancelled
```

Retries must be bounded.

## 27. ADR Requirement

Major architectural changes require an ADR under:

```text
docs/adr/
```

Each ADR should include:

- context;
- decision;
- alternatives;
- consequences.
