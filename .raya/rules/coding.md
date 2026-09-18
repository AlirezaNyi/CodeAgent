# Coding rules

- Idiomatic Rust: explicit ownership, typed errors (`thiserror` in libs).
- No `unwrap()` / `expect()` in production paths.
- Prefer `async` only where needed.
- Avoid unbounded channels and global mutable state.
- Keep changes minimal and task-scoped.
