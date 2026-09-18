# RAYA Agent

Local-first coding agent runtime for software engineering tasks.

Cursor remains the editor UI. RAYA owns orchestration, bounded context, tools, policy, and local persistence.

## Status

Phase 1 (MVP vertical slice) is under active development.

## Requirements

- Rust stable (1.85+)
- macOS or Linux
- Git on PATH (for git tools)

## Quick start

```bash
# Build
cargo build -p raya-cli

# Discover project + show CLI help
cargo run -p raya-cli -- --help

# Example (agent run lands in later tasks)
cargo run -p raya-cli -- agent run "add validation to the login endpoint"
```

Copy [`.raya/config.toml.example`](.raya/config.toml.example) to `.raya/config.toml` to customize limits, policy, and LLM provider.

## Workspace layout

```text
crates/
  raya-core/     # config, errors, discovery, redaction
  raya-cli/      # `raya` binary
docs/            # PRD, RFC, SRS, ADRs
prompts/         # implementation / system prompts
.raya/           # project config example + rules
```

Further crates (`raya-store`, `raya-tools`, `raya-agent`, …) are added as Phase 1 tasks land.

## Documentation

- [Product Requirements (PRD)](docs/PRD.md)
- [Architecture (RFC)](docs/RFC.md)
- [Software Requirements (SRS)](docs/SRS.md)
- [Architecture Decision Records](docs/adr/)
- [Implementation prompt](prompts/IMPLEMENTATION_PROMPT.md)

## Development

```bash
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
```

## License

MIT — see [LICENSE](LICENSE).
