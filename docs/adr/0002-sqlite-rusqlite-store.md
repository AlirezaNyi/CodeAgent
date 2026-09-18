# ADR 0002: SQLite via rusqlite and dedicated store crate

## Status
Accepted

## Context
MVP requires local persistence for projects, tasks, and events without introducing PostgreSQL or other infrastructure.

## Decision
Use `rusqlite` with the bundled SQLite library and `rusqlite_migration` for schema changes. Isolate persistence in a `raya-store` crate. Run blocking SQLite calls via `tokio::task::spawn_blocking`. Enable FTS5 for Phase 2 readiness.

## Alternatives
- `sqlx` with SQLite — heavier async surface for a single-process MVP.
- Connection pool — unnecessary for one local process.

## Consequences
- Sync SQLite API behind a thin async facade.
- `raya-store` is an intentional deviation from the RFC crate list.
