---
name: agent-orchestration
description: >-
  How the main Cursor agent classifies RAYA tasks, picks specialists, parallelizes
  independent work, passes minimal structured handoffs, verifies, retries, and
  escalates to the human. Use for any non-trivial multi-crate or multi-agent task.
---

# RAYA agent orchestration

You (the main agent) are the Lead Orchestrator. Subagents are specialists, not project managers. Do **not** spawn a separate orchestrator subagent.

## 0. Trivial bypass

Handle **directly** (no Task/subagents) when the change is:

- typo / rename / single import / formatting / one-file doc tweak
- clear one-crate edit with obvious fix and no contract risk

## 1. Classify

Tag the request with one or more: `feature` | `bug` | `refactor` | `schema` | `security` | `performance` | `investigation` | `review` | `tooling` | `docs`.

Map risk:

| Risk | Examples |
|------|----------|
| LOW | docs, comments, isolated non-contract tweak |
| MEDIUM | normal API/CLI change, ranking tweak, refactor across 2 crates |
| HIGH | policy defaults, `safe_path`, migrations, TaskPhase/events contracts, auth/bind, tool renames, Phase 3 runtime |

No Docker/K8s/frontend categories — this repo has none.

## 2. Capability routing (existing agents only)

| Agent | Owns | Parallel edits OK with |
|-------|------|------------------------|
| `runtime-safety` | policy, executor, `safe_path`, registry gate, redact | `context-engine`, `persistence` (if no shared files) |
| `persistence` | store, migrations, index **tables** | `runtime-safety`, `context-engine` (symbols only — not schema) |
| `agent-loop` | orchestrator, TaskPhase, events, approvals, planio | `runtime-safety` / `context-engine` if file sets disjoint |
| `context-engine` | ranking, search, symbols, FTS quality | `runtime-safety`; serialize with `persistence` on `raya-index` schema |
| `rust-verifier` | fmt/check/test/clippy + mock smoke | after implementers finish |
| `raya-reviewer` | readonly invariant review | after verifier (or in parallel with docs-only main work) |

Main keeps: CLI, HTTP protocol, LLM providers, docs/ADRs, config, commits.

Built-ins: use `explore` for discovery; use shell for verbose cargo when useful. Prefer specialists over re-discovering the same crates.

## 3. Build a dependency graph

1. Discover (main or `explore`) → short summary of scope + file list.
2. Schema / Store API first when persistence shape changes (`persistence`).
3. Safety boundary before or with new tools (`runtime-safety`).
4. Loop / context implementers consume **handoffs**, not full transcripts.
5. Always: implementers → `rust-verifier` → `raya-reviewer` (MEDIUM+) → main commits.
6. HIGH: also require ADR note / human approval for destructive schema, weaken-deny, or `allow_remote`.

**Parallelize** independent investigations and disjoint file ownership. **Serialize** overlapping file edits (especially `raya-index` schema vs ranking, and `raya-core` models shared by loop + others).

## 4. Minimal handoff (never dump the chat)

Pass each subagent a brief like:

```json
{
  "task": "…",
  "category": ["feature", "schema"],
  "risk": "HIGH",
  "scope": ["crates/raya-store", "migrations"],
  "relevant_files": ["…"],
  "findings": ["users need evidence column already exists — add X"],
  "constraints": ["append-only migrations", "no unwrap in prod"],
  "dependencies": ["persistence output: migration 0003 adds …"],
  "verification_required": true,
  "do_not_commit": true
}
```

Prefer paths, symbols, interfaces, and 5–20 line snippets over whole files.

## 5. Required subagent return shape

Every specialist must answer with:

```markdown
## Summary
## Findings
## Files
## Decisions
## Risks
## Verification
## Recommendations
## Blockers
```

If `Blockers` is non-empty, stop the chain and escalate or re-route.

## 6. Verification gates

```text
Plan → Implement (specialists) → rust-verifier → (fail → owner, max 2 retries) → raya-reviewer → Main (docs/ADR) → commit
```

- Retry limit: **2** fix cycles after verifier failure; then ask the human.
- Never mark complete on implementer claim alone.
- Mock smoke only in `mktemp` projects; never openai for verification.

## 7. Human escalation (ask; do not guess)

- Ambiguous requirements or two architectures with different consequences
- Destructive migration (DROP/rename), weaken deny defaults, non-loopback bind
- Secrets / credentials needed
- Convention conflict with requested change
- Two retries failed or unexplained test behavior
- Product decision (Phase 3 scope, contract breaks)

Do **not** ask questions the repo can answer.

## 8. Git

- Only main agent commits; BEM messages; one atomic commit per completed task
- Never force-push shared branches; do not push unless user asks
- Prefer one branch; use worktrees only if parallel edit streams collide
