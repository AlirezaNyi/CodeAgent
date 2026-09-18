# ADR 0004: Git via bounded shell executor

## Status
Accepted

## Context
Git status/diff/log/commit are required tools. Linking `libgit2` adds build complexity.

## Decision
Invoke the `git` CLI through the bounded process executor (timeout, output limits, cancellation, policy).

## Alternatives
- `git2` / `libgit2` — stronger API, heavier build and ABI concerns.
- Pure Rust git — immature for MVP.

## Consequences
- Requires `git` on PATH (normal for developer machines).
- Reuses executor safety controls.
