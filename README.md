# RAYA Agent

Local-first coding agent runtime for software engineering tasks.

Cursor remains the editor UI. RAYA owns orchestration, bounded context, tools, policy, and local persistence.

## Status

Phases 1–2 are implemented. Phase 3 Slice A (ADR 0011) adds a role-based model router and bounded ephemeral subagents. Slice B (ADR 0012) adds durable approval resume and SQLite project/task memory. Slice C (ADR 0013) adds an optional execution DAG on `plan.nodes`. Slice D (ADR 0014) adds an MCP stdio control plane (`raya mcp`) for Cursor.

`raya agent run` → SQLite task → plan → context → mock/OpenAI LLM → tools → verify → events.

## Requirements

- Rust stable (1.85+)
- macOS or Linux
- Git on PATH (for git tools)

## Quick start

```bash
# Build
cargo build -p raya-cli --release

# Help
cargo run -p raya-cli -- --help

# Run with the offline mock LLM (default)
cargo run -p raya-cli -- agent run "Write a hello.txt file"

# Status / logs
cargo run -p raya-cli -- agent status
cargo run -p raya-cli -- agent logs <task-id>

# Approval resume (after WaitingApproval)
cargo run -p raya-cli -- agent approve <task-id> <call-id>
cargo run -p raya-cli -- agent approve <task-id> <call-id> --deny
cargo run -p raya-cli -- agent resume <task-id>
cargo run -p raya-cli -- agent dag <task-id>

# Memory
cargo run -p raya-cli -- agent memory list
cargo run -p raya-cli -- agent memory search "query"
cargo run -p raya-cli -- agent memory add --kind project "note"
cargo run -p raya-cli -- agent memory forget <id>

# Local HTTP API (127.0.0.1:7319)
cargo run -p raya-cli -- serve

# MCP stdio (Cursor) — blocks on stdin
cargo run -p raya-cli -- mcp
```

Copy [`.raya/config.toml.example`](.raya/config.toml.example) to `.raya/config.toml` to customize limits, policy, memory, and LLM provider.

### Cursor MCP

Add a stdio server entry (adjust the `raya` binary path after `cargo build -p raya-cli --release`):

```json
{
  "mcpServers": {
    "raya": {
      "command": "/absolute/path/to/raya",
      "args": ["mcp", "--project", "/absolute/path/to/your/repo"]
    }
  }
}
```

Do not commit a machine-local `.cursor/mcp.json` with secrets. Control-plane tools only (`raya_run`, `raya_status`, `raya_approve`, memory/dag, …) — filesystem and shell stay inside the agent behind policy (see [ADR 0014](docs/adr/0014-mcp-stdio.md)).

### LLM providers and local lanes

| `llm.provider` | Behavior |
|----------------|----------|
| `mock` | Offline scripted responses (default; used in tests) |
| `openai` / `local` | OpenAI-compatible HTTP (`/chat/completions`) |

`local` is an alias for the same client. Point `[[llm.lanes]]` (or legacy `base_url` / `model`) at Ollama, LM Studio, or any OpenAI-compatible server other agents on your machine already use. Select the active lane with `llm.lane` or `RAYA_LLM_LANE`.

```bash
cargo run -p raya-cli -- agent llm lanes
cargo run -p raya-cli -- agent llm routes
cargo run -p raya-cli -- agent llm probe
cargo run -p raya-cli -- agent llm probe --lane lmstudio
```

Optional `[models]` maps planning/coding/review/debug/summary to lane names. Optional `[subagent] review_on_finish` / `debug_on_verify_fail` spawn Reviewer/Debugger workers (default off).

Loopback hosts (`127.0.0.1`, `localhost`, `::1`) do **not** require an API key (and do not forward ambient `OPENAI_API_KEY`). For non-loopback OpenAI-compatible endpoints, set `RAYA_LLM_API_KEY` (or `OPENAI_API_KEY`). For a loopback server that requires auth, set `api_key_env` on that lane.

See [docs/adr/0010-local-llm-lanes.md](docs/adr/0010-local-llm-lanes.md), [docs/adr/0011-phase3-subagents-and-model-router.md](docs/adr/0011-phase3-subagents-and-model-router.md), [docs/adr/0012-durable-approval-and-memory.md](docs/adr/0012-durable-approval-and-memory.md), [docs/adr/0013-execution-dag.md](docs/adr/0013-execution-dag.md), and [docs/adr/0014-mcp-stdio.md](docs/adr/0014-mcp-stdio.md).

## Workspace layout

```text
crates/
  raya-core/       # config, errors, models, discovery
  raya-store/      # SQLite persistence
  raya-executor/   # bounded process runner
  raya-policy/     # risk classification / approval
  raya-tools/      # filesystem, search, git, shell, test/build
  raya-llm/        # mock + OpenAI-compatible providers + model router
  raya-context/    # search → rank → token budget
  raya-agent/      # orchestrator loop + subagents + resource manager
  raya-protocol/   # localhost HTTP API
  raya-mcp/        # MCP stdio control plane (Cursor)
  raya-cli/        # `raya` binary
docs/              # PRD, RFC, SRS, ADRs
prompts/           # system / planner prompts
.raya/             # config example + rules
```

## Development

```bash
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
```

## License

MIT — see [LICENSE](LICENSE).
