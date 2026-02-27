# Quecto

Rust reimplementation of an agentic personal AI assistant, optimized for ultra-low resource usage.

## Follow Clean Architecture Standards.

```
interface/        --> application/       --> domain/
                      |
infrastructure/  -----+
```

Dependency rule:
- `domain/`: project-internal imports forbidden; only `thiserror`, `serde` (derive), `serde_yaml`
- `application/` -> `domain/` only
- `infrastructure/` -> `domain/` only (implements ports)
- `interface/` composes all layers

Composition root: `main.rs -> cli::run() -> gateway/`.

## Layer summary

### domain/
Pure types + ports: messages/tool calls/usage, provider/tool/agent/session/skill/cron/channel/workspace/voice traits, subagent config validation, and `DomainError`.

Important rule: async trait methods return `Pin<Box<dyn Future + Send + '_>>` (dyn-safe for `Arc<dyn Trait>` usage).

### application/
Use-case orchestration only (no I/O):
- `AgentLoopImpl` tool/LLM loop (`tool_exec` tracing on tool runs)
- Context pruning + spill manifest pipeline
- Onboarding orchestration
- Subagent context inheritance
- Cron tick execution
- Heartbeat parsing/loading/tick execution
- Voice message processing

### infrastructure/
I/O adapters implementing domain ports:
- `config.rs`: serde config + env overrides + exec isolation settings
- `providers/`: OpenAI/Anthropic + `FallbackProvider`; safe `api_base` validation
- `tools/`: exec/fs/spawn/cron/message/web/recall + registry + wasm runtime
- `persistence/`: file/memory stores, session/cron/skills/workspace/spill
- `security/`: sandbox path + command filtering
- `auth/`: credential store + OAuth/device-code flow
- `channels/`: Telegram adapter
- `voice/`: Groq Whisper transcriber
- `logging.rs`: API-key redaction
- `bus.rs`: async message bus
- `health/`: raw tokio TCP liveness/readiness server

## Tool isolation

Two-tier model (no Docker/daemon):
1. **WASM runtime** (`wasm32-wasip2`) for all tools except `exec` + `spawn`
   - Wasmtime component model + WIT bindings
   - Fresh store per call, fuel/memory/epoch limits
   - Host parity checks (workspace bounds, read-size cap, etc.)
2. **nsjail** for `exec` tool
   - Namespace/cgroup/seccomp isolation
   - Configurable strict/fallback behavior via `tools.exec.*`

`spawn` runs `quecto agent` child process inheriting same isolation strategy.
Reference: `reviews/tool-isolation-strategy.md`.

## Development workflow (BDD-first)

Flow:
1. Mark feature `@pending -> @wip`
2. `cargo test --test bdd` (red)
3. Add step defs (still red)
4. Add unit tests (red)
5. Implement
6. `cargo test --lib` + `cargo test --test bdd` (green)
7. Refactor
8. Mark `@wip -> @done`
9. Commit
10. Run 3 reviewer subagents
11. Fix valid findings, push, PR, merge

BDD runner rules:
- Runs `@wip` + `@done`; `.fail_on_skipped()` enabled
- `@pending` excluded
- `@real-llm` only when `QUECTO_REAL_LLM=1` (+ `OPENAI_API_KEY`)
- Optional tag filter: `QUECTO_TAG=<tag>`

## Tech stack

- Rust 2024
- Tokio (rt-multi-thread, macros, signal, time, fs, process)
- reqwest + rustls + stream
- serde/serde_json/serde_yaml
- uuid, chrono, tracing, dirs, thiserror
- Dev: cucumber 0.21, futures, tempfile, wiremock 0.6
