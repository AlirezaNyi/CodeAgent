---
name: add-raya-tool
description: >-
  End-to-end workflow for adding a new RAYA tool: Tool impl, registry,
  policy classify, tests, prompts, docs, and mock/e2e impact. Use when adding
  or renaming a tool in raya-tools.
---

# Add a RAYA tool

Follow this checklist in order. Prefer `runtime-safety` for policy/path work and main agent for CLI/docs commits. Orchestrate via `agent-orchestration` / `feature-development` when the tool also needs loop or prompt contract changes.

## Steps

1. **Implement** `Tool` in `crates/raya-tools/src/tools/<name>.rs`
   - `name()`, `description()`, `schema()` → `ToolSchema`
   - `execute` uses `ToolContext` (project root, cancel, output bounds)
   - Filesystem paths go through `safe_path`
   - Processes go through `raya_executor::run_process`

2. **Export** from `crates/raya-tools/src/tools/mod.rs`.

3. **Register** in `default_registry` in `crates/raya-tools/src/registry.rs`.

4. **Classify** in `raya-policy::classify` (`crates/raya-policy/src/lib.rs`)
   - Map the exact tool name string to a `RiskClass`
   - Add credential/destructive/network heuristics if the tool can reach them
   - Add a policy unit test for allow / approval / deny as appropriate

5. **Prompt** — if the LLM may call it, mention the name in `prompts/system.md` (product template; intentional).

6. **Docs** — update README workspace tool list and `docs/RFC.md` §9 tool list if present.

7. **Contracts**
   - Do not rename existing tools without a migration plan for prompts + mock script
   - Decide whether `MockProvider::default_script` or `crates/raya-agent/tests/e2e_mock_flow.rs` must change

8. **Verify** — run `rust-verifier` (or locally):
   ```bash
   cargo test -p raya-tools -p raya-policy
   cargo clippy --workspace --all-targets --all-features
   ```

9. **Hand off** — main agent commits after review (`raya-reviewer`).

## Tool name contract

Current names: `filesystem.read`, `filesystem.write`, `filesystem.patch`, `search.grep`, `git.status`, `git.diff`, `shell.exec`, `test.run`, `build.run`.
