# Architecture rules

- Prefer small, typed modules over giant orchestrators.
- Lower-level crates must not depend on `raya-agent` or `raya-cli`.
- All tool execution goes through the policy engine.
- Never load the entire repository into an LLM prompt.
- Bound concurrency, iterations, tool calls, and token budgets.
