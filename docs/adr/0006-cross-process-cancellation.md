# ADR 0006: Cross-process task cancellation

## Status
Accepted

## Context
`raya agent cancel` may run in a different process from the agent loop. In-process `CancellationToken` alone is insufficient.

## Decision
Persist `cancel_requested` on the task row. The loop checks the flag between iterations and also listens to an in-process `CancellationToken` for same-process (HTTP) cancels.

## Alternatives
- Unix signals only — not portable across interfaces.
- Shared memory / IPC — overkill for MVP.

## Consequences
- Cancel is eventually consistent across processes.
- HTTP cancel can be immediate via the token.
