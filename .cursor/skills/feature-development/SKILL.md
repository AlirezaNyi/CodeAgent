---
name: feature-development
description: >-
  Cross-crate RAYA feature workflow: classify, plan handoffs, parallel specialists,
  verify, review, commit. Use when adding a capability that spans multiple crates
  (tools, store, loop, context, CLI/HTTP).
---

# Feature development (RAYA)

## Steps

1. **Classify + risk** via `agent-orchestration`. If HIGH (contracts/policy/migrations), plan ADR.
2. **Discover once** — main or `explore`: list crates/files; summarize interfaces. Do not re-explore in every specialist.
3. **Order**
   - Schema / Store API → `persistence` first when needed
   - New/changed tools or process bounds → `runtime-safety`
   - Ranking / index quality → `context-engine` (after schema if index tables change)
   - Task loop / events / decisions → `agent-loop`
   - CLI / HTTP / LLM / docs → **main**
4. **Parallel** only when file ownership is disjoint (e.g. policy tests ‖ ranking tests).
5. **Handoffs** — pass only API/schema deltas between agents (migration name, new columns, tool names, event kinds).
6. **Gate** — `rust-verifier` → fix (≤2) → `raya-reviewer` → main ADR/docs if needed → main commit.
7. Use `add-raya-tool` skill when the feature is primarily a new tool.

## Anti-patterns

- Sequential “backend then frontend” (there is no frontend).
- Sending whole orchestrator.rs to the ranking agent.
- Skipping verifier because “tests were run in the specialist.”
