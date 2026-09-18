# Phase 2 — Repository Intelligence + Observability Conventions

Merged from RFC Phase 2 and AgentTrail A+B recommendations.

## Goals

1. Incremental local index (hashes + FTS5 + symbols).
2. Explainable context ranking (why was this file included?).
3. Evidence-tagged events and human-readable PLAN.md / receipts.

## Deliverables

| Item | Crate / surface |
|------|-----------------|
| `EvidenceKind` on events | `raya-core`, `raya-store`, emit sites |
| `.raya/PLAN.md` sync | `raya-core` planio, orchestrator |
| `raya init` / `activity` / `receipt` | `raya-cli` |
| File hash + FTS5 index | `raya-index`, migration 0002 |
| Ranking + explanations | `raya-context` |
| Symbol extraction (MVP) | `raya-index` (heuristic extractors; Tree-sitter deferred) |

## Non-goals

- AgentTrail Map/Kitchen UI
- Vector DB
- Scraping third-party agent logs as primary truth
