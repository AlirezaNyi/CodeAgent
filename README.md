# RAYA Agent

Local-first coding agent runtime for software engineering tasks.

Cursor remains the editor UI. RAYA owns orchestration, bounded context, tools, policy, and local persistence.

## Status

Phase 1 (MVP vertical slice) is implemented:

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

# Local HTTP API (127.0.0.1:7319)
cargo run -p raya-cli -- serve
```

Copy [`.raya/config.toml.example`](.raya/config.toml.example) to `.raya/config.toml` to customize limits, policy, and LLM provider.

Set `RAYA_LLM_API_KEY` (or `OPENAI_API_KEY`) when `llm.provider = "openai"`.

## Workspace layout

```text
crates/
  raya-core/       # config, errors, models, discovery
  raya-store/      # SQLite persistence
  raya-executor/   # bounded process runner
  raya-policy/     # risk classification / approval
  raya-tools/      # filesystem, search, git, shell, test/build
  raya-llm/        # mock + OpenAI-compatible providers
  raya-context/    # search → rank → token budget
  raya-agent/      # orchestrator loop + resource manager
  raya-protocol/   # localhost HTTP API
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
