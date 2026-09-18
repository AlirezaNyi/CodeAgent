# Agent instructions

## RAYA Agent

- Prefer `raya agent run "..."` for coding tasks in this repo.
- Keep durable plans in `.raya/PLAN.md` (checkbox + `{#id}` convention).
- Sensitive tools may pause for `raya agent approve <task-id> <call-id>`.
- Inspect history with `raya agent logs`, `raya agent activity`, and `raya agent receipt`.

## Repository map

Local-first Rust coding-agent runtime. Cursor is the editor; RAYA owns orchestration, context, tools, policy, and SQLite persistence.

```text
crates/
  raya-core/       # config, errors, models, planio, redact, tokens
  raya-store/      # SQLite + embedded migrations
  raya-executor/   # bounded process runner
  raya-policy/     # risk classify / allow|deny|approval
  raya-tools/      # filesystem, search, git, shell, test, build
  raya-llm/        # mock + OpenAI-compatible providers
  raya-context/    # search → rank → token budget
  raya-index/      # hashes, FTS5, heuristic symbols
  raya-agent/      # orchestrator loop + resources
  raya-protocol/   # localhost axum API (127.0.0.1:7319)
  raya-cli/        # `raya` binary
migrations/        # append-only SQL (embedded by raya-store)
prompts/           # system.md compiled into orchestrator via include_str!
.raya/             # product templates for `raya init` (not Cursor rules)
docs/              # PRD, RFC, SRS, ADRs
```

### Dependency direction

```text
raya-cli / raya-protocol
        ↓
    raya-agent
        ↓
raya-context · raya-llm · raya-tools · raya-index · raya-store
        ↓
raya-policy · raya-executor · raya-core
```

Lower-level crates must not depend on `raya-agent` or `raya-cli`. `raya-context` must not depend on `raya-index` (signals injected via `RankSignals`).

## Quality gates

```bash
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
```

Workspace lints: `unsafe_code = forbid`; `unwrap_used` / `expect_used` = warn.

## Compatibility contracts

Do not break without an intentional, versioned change:

- Tool names: `filesystem.read|write|patch`, `search.grep`, `git.status|diff`, `shell.exec`, `test.run`, `build.run`
- `EventKind::as_str` strings and `EvidenceKind` defaults
- `MockProvider::default_script` (offline e2e / CLI mock provider)
- CLI clap tree and HTTP routes under `/v1/`
- `prompts/system.md` `AgentDecision` JSON schema (`plan` | `tool_call` | `delegate` | `finish` | `needs_fix`)

## Product templates vs Cursor config

`.raya/rules/*.md`, `.raya/config.toml.example`, and `prompts/system.md` are **product templates** shipped by `raya init` / compiled into the binary. Cursor guidance for developing this repo lives in `.cursor/` and this file.

Generated / local-only (never commit): `.raya/raya.db`, `.raya/config.toml`, `.raya/PLAN.md`, `target/`.

## Orchestration (main agent)

The **main Cursor agent** is the Lead Orchestrator. There is no separate orchestrator subagent. Follow skill `agent-orchestration` for non-trivial work.

### Capability matrix

| Agent | Responsibility | Inputs | Outputs | Parallel edits | Depends on |
|-------|----------------|--------|---------|----------------|------------|
| `runtime-safety` | Policy, executor, `safe_path`, registry, redact | Tool name, risk intent, file paths | Code + tests; handoff | Yes with context / persistence if files disjoint | — |
| `persistence` | Store, migrations, index tables | Schema intent | Migration path + Store API delta | Yes with safety; serialize with context on index schema | — |
| `agent-loop` | Orchestrator, TaskPhase, events, approvals, planio | Store API / tool contracts | Loop code + e2e | Yes if files disjoint | Store API when schema changes |
| `context-engine` | Rank, search, symbols, FTS quality | RankSignals contract, schema version | Ranking/index quality code | Yes with safety; after persistence for schema | Persistence for table shape |
| `rust-verifier` | Gates + mock smoke | Touched crates | Pass/fail + owners | After implementers | Implementers |
| `raya-reviewer` | Readonly invariant review | Diff | Critical/Warning/Suggestion | After verifier | Verifier (preferred) |

Main keeps: CLI, HTTP, LLM providers, docs/ADRs, config, **all commits**.

### When not to delegate

Typos, renames, single import, formatting, tiny one-file docs — main agent only.

### Default pipeline

```text
Discover (once) → specialists (parallel if disjoint) → rust-verifier
  → (fail → owner ≤2 retries) → raya-reviewer → main docs/ADR → commit
```

### Risk

- **LOW** — docs / isolated tweak → optional review
- **MEDIUM** — multi-crate / API → verifier + reviewer
- **HIGH** — policy defaults, migrations, TaskPhase/events contracts, path/executor → verifier + reviewer + human before destructive actions

### Skills

| Skill | When |
|-------|------|
| `agent-orchestration` | Any non-trivial multi-agent task |
| `feature-development` | Cross-crate features |
| `bug-fix` | Failures / wrong behavior |
| `schema-change` | Migrations / store / index tables |
| `add-raya-tool` | New or renamed tool |

### Handoffs

Pass JSON briefs (task, scope, files, findings, constraints). Subagents return Summary / Findings / Files / Decisions / Risks / Verification / Recommendations / Blockers.

## Delegation quick map

| Area | Subagent |
|------|----------|
| Policy, executor bounds, `safe_path`, registry gate, redaction | `runtime-safety` |
| Store, migrations, index tables | `persistence` |
| Orchestrator, TaskPhase, events, approvals, Phase 3 runtime | `agent-loop` |
| Ranking, search, symbols, FTS | `context-engine` |
| cargo fmt/check/test/clippy + mock smoke | `rust-verifier` |
| Diff review against project invariants | `raya-reviewer` |

## Commits

Only the main agent creates commits. Format: `block: description` or `block__element: description` (lowercase, imperative). One atomic commit per completed task. Never force-push shared branches.
