# ADR 0003: In-process repository search

## Status
Accepted

## Context
Lexical search is required for the context engine. A standalone `rg` binary is not guaranteed on PATH.

## Decision
Use ripgrep library crates (`ignore`, `grep-searcher`, `grep-regex`) in-process with match and file-size bounds, honoring `.gitignore`.

## Alternatives
- Shell out to `rg` — PATH and version fragility.
- Walk + `regex` only — slower and weaker ignore handling.

## Consequences
- No external `rg` dependency for MVP.
- Search behavior is deterministic and bounded.
