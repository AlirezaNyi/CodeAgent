# ADR 0009: Phase 2 + AgentTrail observability conventions

## Status
Accepted

## Context
Phase 1 delivered the MVP vertical slice. Phase 2 calls for repository intelligence (hash index, FTS5, symbols, ranking). Separately, review of [sodiumsun/agenttrail](https://github.com/sodiumsun/agenttrail) identified low-cost observability conventions that fit RAYA without adopting its Map/Kitchen UI.

## Decision
Merge AgentTrail-inspired A+B into Phase 2:

1. **EvidenceKind** on every event: `reported` | `observed` | `inferred` | `unknown`.
2. **PLAN.md** at `.raya/PLAN.md` — export/import of `ExecutionPlan` using AgentTrail-compatible checkbox / `{#id}` conventions where practical.
3. **CLI**: `raya init`, `raya agent activity`, `raya agent receipt <task-id>`.
4. **raya-index**: file content hashes, incremental reindex, SQLite FTS5 chunks, optional symbol rows.
5. **Context ranking**: add path/symbol/git/recency signals and persist rank explanations on `context.retrieved`.

Do **not** port AgentTrail Kitchen/Map UI or depend on Claude/Codex log scraping as the primary event source. RAYA remains the system of record via SQLite events.

## Alternatives
- Ship Phase 2 index only — misses quick DX wins from PLAN.md/receipts.
- Embed AgentTrail as a companion — dual backends and weaker trust in our events.

## Consequences
- Migration `0002` extends `events` and adds index tables.
- New crate `raya-index`; context engine may query FTS when an index exists.
- Agents and humans can read `.raya/PLAN.md` from Cursor without a custom UI.
