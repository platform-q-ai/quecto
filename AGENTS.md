# Quecto

Quecto is a Rust reimplementation of [PicoClaw](../picoclaw) — a Go-based personal AI assistant — rebuilt from scratch to target ultra-low resource usage. It keeps the core assistant architecture (agent loop, tool use, provider fallback, session persistence, Telegram interface) but drops hardware integrations, most provider backends, and multi-platform support.

## What we want to achieve

A single static binary that runs an autonomous AI agent on minimal Linux systems (VPS, Raspberry Pi, containers). The agent:

- Receives messages via Telegram (or CLI)
- Routes them through an LLM provider (OpenAI or Anthropic, with automatic fallback)
- Executes tools (shell commands, file operations, web search, cron scheduling, subagent spawning)
- Persists sessions and cron jobs to disk
- Loads user-defined skills from the filesystem
- Runs a heartbeat loop for scheduled tasks
- Stays within a security sandbox that restricts file access and command execution

Linux only. No clap. No hardware. No migration tooling. English only.

## Architecture

Four layers with strict dependency direction. Inner layers never import from outer layers.

```
interface/        --> application/       --> domain/
                      |
infrastructure/  -----+
```

### domain/ — Pure types and traits

Zero external dependencies except `thiserror`. Defines the vocabulary of the system:

| File | Purpose |
|---|---|
| `message.rs` | `Message`, `Role`, `ToolCall`, `LlmResponse`, `UsageInfo` |
| `provider.rs` | `LlmProvider` trait (dyn-compatible via `Pin<Box<dyn Future>>`) |
| `tool.rs` | `Tool` trait, `ToolRegistry` trait, `ToolDefinition`, `ToolResult` |
| `agent.rs` | `AgentLoop` trait, `AgentInfo`, `AgentResult` |
| `session.rs` | `Session`, `SessionStore` trait |
| `skill.rs` | `Skill`, `SkillSource`, `SkillLoader` trait |
| `cron.rs` | `CronJob`, `CronSchedule`, `CronStore` trait |
| `channel.rs` | `Channel` trait |
| `subagent.rs` | `SubagentConfig`, `validate_agent_id()` |
| `error.rs` | `DomainError` enum (Provider, Tool, Session, Channel, Security, Config, Other) |

Traits in this layer define ports. They use `Pin<Box<dyn Future + Send + '_>>` return types (not `impl Future`) so they can be used as `Arc<dyn Trait>` in registries and collections.

### application/ — Use cases

Depends only on `domain/`. Contains orchestration logic with no I/O — all I/O goes through trait ports.

| File | Purpose |
|---|---|
| `agent_loop.rs` | `AgentLoopImpl` — the core LLM-tool loop: send messages to provider, execute tool calls, repeat until done or max iterations |
| `onboard.rs` | `run_onboard()` — creates workspace directory, writes default config and template files |
| `subagent.rs` | `SubagentContext` — constructs child agent contexts with inherited sandbox restrictions (re-exports `SubagentConfig` from domain) |
| `heartbeat.rs` | `parse_heartbeat()`, `load_tasks()` — parses cron-like task definitions, determines which tasks are due |

### infrastructure/ — Concrete adapters

Implements the domain traits with real I/O. This is where serde, reqwest, tokio, and filesystem operations live.

| Directory | Contents |
|---|---|
| `config.rs` | `Config` struct with serde deserialization, env var overrides, workspace path expansion |
| `providers/` | `OpenAiProvider`, `AnthropicProvider` (real HTTP), `FallbackProvider` (cooldown + error classification), `ErrorClass` |
| `tools/` | `ExecTool` (shell), `ReadFileTool`/`WriteFileTool`/`EditFileTool`/`AppendFileTool`/`ListDirTool` (filesystem), `SpawnTool` (subagent), `CronTool`, `MessageTool`, `WebSearchTool` (Brave + DDG), `ToolRegistryImpl` |
| `persistence/` | `FileSessionStore`, `MemoryStore`, `FileCronStore`, `FileSkillLoader` |
| `security/` | `Sandbox` — workspace path validation and command filtering |
| `auth/` | `CredentialStore` (file-based token CRUD), `oauth.rs` (stub) |
| `channels/` | `TelegramChannel` — `send_message()`, `get_updates()`, user allowlist |
| `voice/` | `GroqWhisperClient` — speech-to-text via Groq API |
| `bus.rs` | `MessageBus` — async channel for inbound/outbound message passing |
| `health/` | Health check server (stub) |

### interface/ — CLI + Gateway (composition root)

Manual argument parsing (no clap). The single entry point is `cli::run(args) -> i32`. The gateway lives here because it wires concrete infrastructure types together (composition root).

| Command | What it does |
|---|---|
| `quecto onboard` | Creates workspace and default config |
| `quecto agent [-s system] [-m model] <prompt>` | Runs a one-shot agent session |
| `quecto skills list\|remove\|install` | Manages skill files |
| `quecto status` | Shows config summary, provider availability, redacted API keys |
| `quecto auth login --provider <name> --token <key>` | Stores an API token for a provider in the credential store |
| `quecto auth logout --provider <name>` | Removes a stored credential (no-op if absent) |
| `quecto auth status` | Lists all stored credentials with provider, method, and active/expired status |
| `quecto gateway` | Runs the full async gateway (Telegram polling + agent loop) |
| `quecto help` / `quecto version` | Self-explanatory |

`CliContext` allows overriding `base_dir` for testability so commands write to temp directories in tests instead of `~/.config/quecto`.

The gateway module (`gateway.rs`) also provides credential-store integration as free functions:

- `resolve_api_key(config_key, creds, provider)` — given a pre-loaded credential snapshot, returns the store token if present and not expired, otherwise falls back to the config file key
- `check_provider_readiness(creds)` — given a pre-loaded credential snapshot, returns a list of providers whose stored credentials have expired and need re-authentication

Both functions operate on a `HashMap<String, Credential>` snapshot (from `CredentialStore::load_snapshot()`) to avoid redundant file I/O. The gateway calls `load_snapshot()` once at startup and passes the result to both functions.

## Dependency rule

```
interface/  -->  application/  -->  domain/
     |                |
     +--- infrastructure/ ---+
```

- `domain/` imports nothing from the project. Only `thiserror`.
- `application/` imports `domain/` only. Never `infrastructure/`.
- `infrastructure/` imports `domain/` (to implement traits). Never `application/`.
- `interface/` imports all three to wire things together (composition root).

The composition root is `main.rs` -> `cli::run()` -> `gateway.rs`. The gateway constructs all concrete types and passes them as `Arc<dyn Trait>` to the application layer.

## Development workflow

BDD-first using cucumber-rs with Gherkin feature files.

```
@pending -> @wip    Tag the feature
cargo test --test bdd       FAIL (skipped steps)
Write step definitions      FAIL (logic missing)
Write unit tests            FAIL (red)
Implement production code
cargo test --lib            PASS (green)
cargo test --test bdd       PASS (green)
Refactor
@wip -> @done       Tag the feature
```

The BDD runner (`tests/bdd.rs`) uses `.fail_on_skipped()` and runs features tagged `@wip` or `@done`. This means all completed features are regression-tested on every run. Scenarios tagged `@pending` are always excluded. All step definitions live in `tests/bdd.rs` (~3600 lines).

Feature files live in `tests/features/`. There are 17 feature files covering: config, cli, onboard, security, sandbox_hardening, agent_tools, providers, agent_loop, session, auth, telegram, cron, subagent, heartbeat, skills, voice, observability.

## Quality gates

```
cargo fmt --check              Formatting
cargo clippy -- -D warnings    Lints (zero warnings policy)
cargo test --lib               295 unit tests
cargo test --test bdd          134 active BDD scenarios across 17 @done features
```

## Tech stack

- Rust 2024 edition
- Tokio async runtime (rt-multi-thread, macros, signal, time, fs, process)
- reqwest with rustls-tls (no OpenSSL dependency)
- serde/serde_json for config and API payloads
- uuid, chrono, tracing, dirs, thiserror
- Dev: cucumber 0.21, futures, tempfile, wiremock 0.6
