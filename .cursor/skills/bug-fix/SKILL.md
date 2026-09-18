---
name: bug-fix
description: >-
  RAYA bug workflow: reproduce with smallest crate tests or mock smoke, route to
  owning specialist, verify, review. Use for runtime failures, wrong TaskPhase,
  policy misses, store bugs, ranking/index incorrectness.
---

# Bug fix (RAYA)

## Steps

1. **Reproduce** with the smallest signal:
   - unit/integration: `cargo test -p <crate> <test_name>`
   - loop e2e: `raya-agent` mock flow / temp-dir `raya agent run` with `provider=mock`
   - Prefer failing test over prose speculation.
2. **Own the fault**
   - policy / path / kill / redact → `runtime-safety`
   - SQLite / migration / index tables → `persistence`
   - phase / approval / events / resources → `agent-loop`
   - ranking / FTS / symbols → `context-engine`
   - CLI / HTTP / LLM client → main
3. **Implement** minimal fix in the owner; do not “clean up” unrelated code.
4. **Known defects** — do not silently rewrite approval-resume or ignored `add_tokens` unless the bug task is about them; note if adjacent.
5. **Verify** — `rust-verifier` (targeted then workspace if needed). Retry owner ≤2 times.
6. **Review** — `raya-reviewer` for MEDIUM+ or any safety/persistence change.
7. Main commits.

## Investigation-only

If the user only asks “why”: use `explore` or readonly specialist; no verifier gate unless they ask to fix.
