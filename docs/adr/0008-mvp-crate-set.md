# ADR 0008: MVP crate set (defer empty crates)

## Status
Superseded in part by ADR 0009 / Phase 2

## Context
The RFC lists crates including index, subagent, memory, and MCP that are Phase 2/3 features.

## Decision
Implement only crates with real MVP content initially. Phase 2 adds `raya-index` with real hashing/FTS/symbol code (see ADR 0009). Still defer `raya-subagent`, `raya-memory`, `raya-mcp` until Phase 3.

## Alternatives
- Create all RFC crates empty — noise and false completeness.

## Consequences
- Cleaner dependency graph for MVP.
- `raya-index` is now a real workspace member.
