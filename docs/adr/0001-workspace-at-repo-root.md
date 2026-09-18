# ADR 0001: Workspace at repository root

## Status
Accepted

## Context
The RFC shows a nested `raya-agent/` directory. This repository is already named for the product and currently holds only specification documents.

## Decision
Place the Cargo workspace at the repository root. Move PRD/RFC/SRS into `docs/` and the implementation prompt into `prompts/`.

## Alternatives
- Nested `raya-agent/` subdirectory — adds an unnecessary path layer.
- Separate monorepo layout — premature for a single product.

## Consequences
- `cargo` commands run from the repo root.
- Spec paths become `docs/PRD.md`, `docs/RFC.md`, `docs/SRS.md`.
