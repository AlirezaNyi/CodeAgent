# ADR 0008: MVP crate set (defer empty crates)

## Status
Accepted

## Context
The RFC lists crates including index, subagent, memory, and MCP that are Phase 2/3 features.

## Decision
Implement only crates with real MVP content: `raya-core`, `raya-store`, `raya-policy`, `raya-executor`, `raya-tools`, `raya-llm`, `raya-context`, `raya-agent`, `raya-protocol`, `raya-cli`. Do not scaffold empty placeholders.

## Alternatives
- Create all RFC crates empty — noise and false completeness.

## Consequences
- Cleaner dependency graph.
- Phase 2 adds crates when they have real code.
