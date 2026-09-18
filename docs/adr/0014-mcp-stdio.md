# ADR 0014: MCP stdio control plane (`raya-mcp`)

## Status
Accepted

## Context
RFC §3 lists `raya-mcp` beside `raya-cli` / `raya-protocol` as a top-level surface into `raya-agent`. Slices A–C delivered subagents, durable approval/memory, and an optional execution DAG. Cursor still needed a first-class MCP entry without exposing filesystem/shell tools over MCP or inventing new `TaskPhase` / `EventKind` / migrations. ADR 0008 deferred an empty stub; this slice ships a real crate.

## Decision
1. **New workspace crate `raya-mcp`.** Depends on `raya-agent` / Store / tools / policy / LLM — not an empty stub. CLI owns `raya mcp`; protocol stays HTTP-only.
2. **Transport: stdio** via official Rust SDK `rmcp` (server + macros + transport-io). Cursor wires `.cursor/mcp.json` to `raya mcp`. Streamable HTTP MCP is Phase 4.
3. **In-process control plane only.** Same wiring as CLI (`Store` → `PolicyEngine` → `ToolRegistry` → `ModelRouter` → `Orchestrator`). Do not spawn `raya serve` or reimplement tools.
4. **Stable tool names:** `raya_run`, `raya_status`, `raya_logs`, `raya_approve`, `raya_resume`, `raya_cancel`, `raya_dag`, `raya_memory_list|search|add|forget`. No `filesystem.*` / `shell.exec` on MCP — those remain inside the agent behind `PolicyEngine`.
5. **Reuse Slice B paths.** Approve/resume use checkpoint + `set_approval` + `Orchestrator::run`. Responses pass through `redact_secrets`; list limits clamp to 100.
6. **Local-first.** Stdio only — no remote bind. Loopback `[server]` defaults are irrelevant for this surface. Tracing for `raya mcp` writes to **stderr** so JSON-RPC on stdout stays clean.

## Alternatives
- Fold MCP into `raya-protocol` as an axum module — mixes HTTP and stdio concerns; Cursor’s primary MCP transport is stdio.
- Expose RAYA filesystem/shell tools on MCP — duplicates Cursor’s tools and bypasses the intended control-plane boundary.
- HTTP Streamable MCP first — deferred to Phase 4 with richer Cursor extension work.

## Consequences
- Operators can drive run/approve/memory/DAG from Cursor MCP without leaving the editor.
- Phase 4 leftovers: Cursor extension, Streamable HTTP MCP, Web UI.
- Ownership stays with the main agent (CLI + docs); `agent-loop` does not own MCP glue.
