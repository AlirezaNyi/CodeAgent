---
name: schema-change
description: >-
  Append-only SQLite / index schema workflow for RAYA. Use when adding migrations,
  changing store tables, or altering indexed_files/symbols/FTS shape.
---

# Schema change (RAYA)

Risk: usually **HIGH**. Ask the human before DROP/rename or touching a real `.raya/raya.db`.

## Steps

1. Delegate implementation to `persistence` with a brief stating the desired columns/tables and consumers.
2. **Append-only**
   - New `migrations/000N_*.sql` only
   - Wire `include_str!` + `M::up` in `crates/raya-store/src/store.rs`
   - Never edit `0001_init.sql` / `0002_index.sql` (hook-denied)
3. If index row shape changes: bump `index_version`; coordinate FTS/hash behavior with `context-engine` **after** schema lands (serialize).
4. If event/evidence contracts change: notify `agent-loop` + plan ADR (main writes ADR).
5. Tests: reopen/roundtrip in store; index tests if applicable.
6. `rust-verifier` → `raya-reviewer` → main commit.

## Handoff to dependents

Return only:

```text
Migration: migrations/000N_….sql
Store API: <new methods/fields>
Consumers to update: [agent-loop | index | protocol]
```

Do not paste full SQL into every downstream agent — path + summary is enough.
