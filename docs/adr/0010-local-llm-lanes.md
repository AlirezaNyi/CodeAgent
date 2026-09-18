# ADR 0010: Local LLM lanes

## Status
Accepted

## Context
RAYA needs to reuse OpenAI-compatible backends already running on the developer machine (Ollama, LM Studio, local proxies) without requiring a cloud API key. A single top-level `base_url` / `model` pair is awkward when several local agents share different endpoints.

Cursor cloud subscription models are not callable as a third-party chat-completions API; lanes only target HTTP OpenAI-compatible servers.

## Decision
1. Add named `[[llm.lanes]]` entries (`name`, `base_url`, `model`, optional `api_key_env`).
2. Select the active lane via `llm.lane` or `RAYA_LLM_LANE`; empty `lanes` keeps legacy top-level fields.
3. Allow `llm.provider = "local"` as an alias for the existing OpenAI-compatible client.
4. If the active `base_url` is loopback (`127.0.0.1`, `localhost`, `::1`), use placeholder key `"local"` and do **not** forward ambient `OPENAI_API_KEY` / `RAYA_LLM_API_KEY`. To send a key to a loopback server, set `api_key_env` on that lane.
5. Expose `raya agent llm lanes` and `raya agent llm probe` for listing and best-effort `/models` discovery.

## Alternatives
- Auto-scrape Cursor settings for model IDs — fragile and cannot access Cursor cloud models.
- Native Ollama / Anthropic clients — deferred; OpenAI-compatible cover is enough for local MVPs.

## Consequences
- Config contract expands; `mock` offline path unchanged.
- Phase 3 phase-based model router can build on named lanes later.
