# ADR 0007: Heuristic token counter

## Status
Accepted

## Context
Context budgets must be enforced before LLM calls. Exact tokenization depends on the model.

## Decision
Introduce a `TokenCounter` trait with a `HeuristicCounter` (approximately `chars / 4`) for MVP.

## Alternatives
- `tiktoken` — model-specific and heavier.
- No budget — violates NFR-003.

## Consequences
- Budgets are approximate but enforced.
- Counter can be swapped later without API breakage.
