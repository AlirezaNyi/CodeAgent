# ADR 0005: HTTP and CLI stack

## Status
Accepted

## Context
MVP needs a localhost HTTP API and a developer CLI.

## Decision
Use `axum` 0.8 for HTTP, `clap` 4 derive for CLI, and `reqwest` (rustls + json) for the OpenAI-compatible LLM client.

## Alternatives
- `actix-web` — more complex than needed.
- Manual `hyper` routing — unnecessary boilerplate.

## Consequences
- Standard Tokio-friendly stack.
- Default bind `127.0.0.1:7319`.
