# Quecto

Quecto is a Rust reimplementation of an agentic personal AI assistant — built from scratch to target ultra-low resource usage. It keeps the core assistant architecture (agent loop, tool use, provider fallback, session persistence, Telegram interface).

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

Zero external dependencies except `thiserror`, `serde` (derive only), and `serde_yaml` (for skill frontmatter parsing). Defines the vocabulary of the system:

| File | Purpose |
|---|---|
| `message.rs` | `Message`, `Role`, `ToolCall`, `LlmResponse`, `UsageInfo` |
| `provider.rs` | `LlmProvider` trait (dyn-compatible via `Pin<Box<dyn Future>>`), `chat()` + `chat_stream()` (SSE streaming with non-streaming fallback) |
| `tool.rs` | `Tool` trait, `ToolRegistry` trait, `ToolDefinition`, `ToolResult` |
| `agent.rs` | `AgentLoop` trait, `AgentInfo`, `AgentResult` |
| `session.rs` | `Session`, `SessionStore` trait |
| `skill.rs` | `Skill`, `SkillSource`, `SkillFrontmatter`, `SkillLoader` trait, `parse_skill_md()`, `is_valid_skill_name()` |
| `cron.rs` | `CronJob`, `CronJobResult`, `CronSchedule`, `CronStore` trait |
| `channel.rs` | `Channel` trait |
| `subagent.rs` | `SubagentConfig`, `validate_agent_id()` |
| `voice.rs` | `VoiceTranscriber` trait, `TranscriptionResult`, `TranscriptionError` |
| `error.rs` | `DomainError` enum (Provider, Tool, Session, Channel, Security, Config, Other) |

Traits in this layer define ports. They use `Pin<Box<dyn Future + Send + '_>>` return types (not `impl Future`) so they can be used as `Arc<dyn Trait>` in registries and collections.

### application/ — Use cases

Depends only on `domain/`. Contains orchestration logic with no I/O — all I/O goes through trait ports.

| File | Purpose |
|---|---|
| `agent_loop.rs` | `AgentLoopImpl` — the core LLM-tool loop: send messages to provider, execute tool calls, repeat until done or max iterations. Emits `tracing::info!` on each tool execution with `tool_name`, `duration_ms`, `is_error` fields (target: `tool_exec`) |
| `onboard.rs` | `run_onboard()` — creates workspace directory, writes default config and template files |
| `subagent.rs` | `SubagentContext` — constructs child agent contexts with inherited sandbox restrictions (re-exports `SubagentConfig` from domain) |
| `cron_executor.rs` | `execute_cron_tick()` — runs due cron jobs through the agent with timeout, records `last_error` on failure, propagates `deliver_to` |
| `heartbeat.rs` | `parse_heartbeat()`, `load_tasks()`, `execute_heartbeat_tick()` — parses task definitions, determines which are due, dispatches tasks through the agent (spawn-aware) |
| `voice.rs` | `process_voice_message()` — transcribes audio via `VoiceTranscriber` and routes the text through the agent, returns `VoiceProcessingResult` |

### infrastructure/ — Concrete adapters

Implements the domain traits with real I/O. This is where serde, reqwest, tokio, and filesystem operations live.

| Directory | Contents |
|---|---|
| `config.rs` | `Config` struct with serde deserialization, env var overrides, workspace path expansion |
| `providers/` | `OpenAiProvider`, `AnthropicProvider` (real HTTP, SSE streaming via `chat_stream()`), `FallbackProvider` (cooldown + error classification), `ErrorClass` |
| `tools/` | `ExecTool` (shell), `ReadFileTool`/`WriteFileTool`/`EditFileTool`/`AppendFileTool`/`ListDirTool` (filesystem), `SpawnTool` (subagent), `CronTool`, `MessageTool`, `WebSearchTool` (Brave + DDG), `ToolRegistryImpl` |
| `persistence/` | `FileSessionStore`, `MemoryStore`, `FileCronStore`, `FileSkillLoader` (single workspace path, YAML frontmatter validation) |
| `security/` | `Sandbox` — workspace path validation and command filtering |
| `auth/` | `CredentialStore` (file-based token CRUD), `oauth.rs` (`OAuthConfig`, `DeviceCodeResponse`, `request_device_code()` — OAuth browser flow and device code flow for headless environments) |
| `channels/` | `TelegramChannel` — `send_message()`, `get_updates()`, user allowlist, configurable `api_base` (defaults to `https://api.telegram.org`) |
| `voice/` | `GroqWhisperClient` — speech-to-text via Groq API, implements `VoiceTranscriber` trait |
| `logging.rs` | `redact_api_keys()` — pattern-based API key redaction for tracing output (matches `sk-*`, `sk-ant-*` prefixes) |
| `bus.rs` | `MessageBus` — async channel for inbound/outbound message passing |
| `health/` | `HealthServer` — lightweight HTTP health server using raw tokio TCP (no hyper/axum). `ReadinessCheck` trait, `StaticReadiness`. Endpoints: `/health` (liveness, always 200), `/ready` (readiness, 200 or 503 based on provider availability) |

### interface/ — CLI + Gateway (composition root)

Manual argument parsing (no clap). The single entry point is `cli::run(args) -> i32`. The gateway lives here because it wires concrete infrastructure types together (composition root).

| Command | What it does |
|---|---|
| `quecto` (no args) | Enters interactive REPL mode (see below) |
| `quecto onboard` | Creates workspace and default config |
| `quecto agent -m <message> [-s <session>] [--system <prompt>] [--model <model>] [--max-iterations <n>] [--max-time <secs>]` | Runs a headless one-shot agent session (see below) |
| `quecto skills list\|remove\|install` | Manages skill files |
| `quecto status` | Shows config summary, provider availability, redacted API keys |
| `quecto auth login --provider <name> [--token <key>] [--oauth] [--device-code]` | Authenticates with a provider: paste token interactively (default), pass `--token` directly, `--oauth` for browser flow, or `--device-code` for headless environments |
| `quecto auth logout --provider <name>` | Removes a stored credential (no-op if absent) |
| `quecto auth status` | Lists all stored credentials with provider, method, and active/expired status |
| `quecto gateway` | Runs the full async gateway (Telegram polling + agent loop) |
| `quecto help` / `quecto version` | Self-explanatory |

`CliContext` allows overriding `base_dir` for testability so commands write to temp directories in tests instead of `~/.config/quecto`. Additional fields: `stdin_data` (pre-loaded stdin for testing interactive commands like token paste) and `oauth_base_url` (override OAuth endpoints for testing with wiremock). Base directory resolution order: explicit `CliContext.base_dir` override > `QUECTO_BASE_DIR` environment variable > platform default.

#### `quecto` — Interactive REPL mode

Runs an interactive read-eval-print loop. The REPL reads user input line by line, sends each to the LLM agent, prints the response, and repeats. Optional flags:

| Flag | Description |
|---|---|
| `-s` / `--session` | Session name for persistence. Default: `repl:repl_default`. Use `-` for ephemeral |
| `--system` | System prompt prepended to each turn (not persisted) |
| `--model` | Override the default model from config |

REPL commands: `/help` (show commands), `/clear` (reset history), `/cron` (manage cron jobs), `/heartbeat` (manage heartbeat tasks), `/agent` (manage subagent profiles), `/spawn` (spawn child agent tasks), `/exit` or `/quit` (exit). Ctrl+D (EOF) also exits cleanly.

The REPL uses abstracted I/O (`BufRead` + `Write` traits) instead of hardcoded stdin/stdout. This allows:
- Interactive terminal use (stdin/stdout with TTY detection for prompt/banner)
- Piped input for scripting (`echo "hello" | quecto`)
- In-memory buffers for BDD testing (`run_repl_with_output()`)

Production code lives in `src/interface/repl/`. Key types: `ReplLoop<R, W>` (the generic loop), `ReplContext` (config + provider bundle), `ReplFlags` (parsed CLI flags), `ReplSession` (agent + persistence state). The module is split into `mod.rs` (core loop), `parsers.rs` (input parsing), `cmd_agent.rs` (agent profile management), `cmd_cron.rs` (cron commands), `cmd_heartbeat.rs` (heartbeat commands), and `cmd_spawn.rs` (spawn commands).

#### `quecto agent` — Headless one-shot mode

Runs a full agent cycle (LLM call → tool execution → repeat) for a single message and exits. Flags:

| Flag | Required | Description |
|---|---|---|
| `-m` / `--message` | Yes | The user message to process |
| `-s` / `--session` | No | Session name for persistence. Omit for `cli:default`. Use `-` for ephemeral (no persistence) |
| `--system` | No | System prompt prepended to conversation (not persisted in session history) |
| `--model` | No | Override the default model from config |
| `--max-iterations` | No | Override max tool iterations (takes precedence over config `max_tool_iterations`) |
| `--max-time` | No | Wall-clock timeout in seconds for the entire agent run. Exit code 2 on timeout |

The agent loads config from `<base_dir>/config.json`, builds a `FallbackProvider` from configured credentials, constructs the tool registry with sandbox enforcement, and runs the `AgentLoopImpl`. Sessions are loaded from and saved to `<base_dir>/sessions/` via `FileSessionStore`. Workspace skills (from `<base_dir>/workspace/skills/`) are loaded at startup and their body content (everything after the YAML frontmatter closing `---`) is prepended to the system prompt (combined with `--system` if provided). Skills with missing or invalid frontmatter are silently skipped.

The CLI module (`src/interface/cli/`) is split into `mod.rs` (entry point, dispatch, REPL wiring), `agent.rs` (agent subcommand, flag parsing, session management), `auth.rs` (auth login/logout/status flows), and `commands.rs` (onboard, status, skills, help, version).

The gateway module (`src/interface/gateway/`) is split into `mod.rs` (Gateway struct, event loop, startup), `telegram.rs` (Telegram polling, update dispatch, bot commands), and `services.rs` (credential resolution, provider building, whisper client construction). It also provides credential-store integration and bot command handling as free functions:

- `resolve_api_key(config_key, creds, provider)` — given a pre-loaded credential snapshot, returns the store token if present and not expired, otherwise falls back to the config file key
- `check_provider_readiness(creds)` — given a pre-loaded credential snapshot, returns a list of providers whose stored credentials have expired and need re-authentication
- `handle_bot_command(text, config)` — intercepts known Telegram bot commands (`/start`, `/help`, `/status`) and returns a response string. Returns `None` for unknown commands, which are forwarded to the agent as regular text. Called by `dispatch_update()` before routing messages to the inbound channel.

Both credential functions operate on a `HashMap<String, Credential>` snapshot (from `CredentialStore::load_snapshot()`) to avoid redundant file I/O. The gateway calls `load_snapshot()` once at startup and passes the result to both functions.

The `EventLoopContext` struct holds the runtime state for the gateway's `tokio::select!` event loop: inbound/outbound channels, agent, session store, Telegram channel, and `Config`. The `Config` field is passed through the polling chain (`run_telegram_polling` → `poll_once` → `dispatch_update`) so that bot commands can access configuration values (e.g. current model name for `/status`). Graceful shutdown is handled by `tokio::select!` — when `ctrl_c()` fires, all branches are dropped and channel receivers return `None`, exiting loops cleanly without errors.

## Dependency rule

```
interface/  -->  application/  -->  domain/
     |                |
     +--- infrastructure/ ---+
```

- `domain/` imports nothing from the project. Only `thiserror`, `serde` (derive), and `serde_yaml`.
- `application/` imports `domain/` only. Never `infrastructure/`.
- `infrastructure/` imports `domain/` (to implement traits). Never `application/`.
- `interface/` imports all three to wire things together (composition root).

The composition root is `main.rs` -> `cli::run()` -> `gateway/`. The gateway constructs all concrete types and passes them as `Arc<dyn Trait>` to the application layer.

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

The BDD runner (`tests/bdd/main.rs`) uses `.fail_on_skipped()` and runs features tagged `@wip` or `@done`. This means all completed features are regression-tested on every run. Scenarios tagged `@pending` are always excluded. Scenarios tagged `@real-llm` are excluded unless `QUECTO_REAL_LLM=1` is set (requires `OPENAI_API_KEY` via env var or `.env` file). Set `QUECTO_TAG=<tag>` to run only scenarios matching a specific tag (e.g. smoke: `timeout 5m env QUECTO_REAL_LLM=1 QUECTO_TAG=real-llm-smoke cargo test --test bdd`, full: `timeout 5m env QUECTO_REAL_LLM=1 QUECTO_TAG=real-llm cargo test --test bdd`). Step definitions live in `tests/bdd/` split across 19 module files (~9000 lines total).

Feature files live in `tests/features/`. There are 44 feature files covering: agent_cli, agent_loop, agent_tools, auth, cli, config, cron, e2e_agentic_loop, e2e_gateway_cron, e2e_gateway_health, e2e_gateway_heartbeat, e2e_gateway_spawn, e2e_gateway_voice, e2e_providers, e2e_real_llm, e2e_real_llm_agent_matrix, e2e_real_llm_entrypoints, e2e_real_llm_entrypoints_matrix, e2e_real_llm_gateway, e2e_real_llm_gateway_matrix, e2e_real_llm_repl, e2e_real_llm_repl_matrix, e2e_repl_agents, e2e_repl_cron, e2e_repl_heartbeat, e2e_repl_spawn, e2e_safety, e2e_session, e2e_session_tools, e2e_skills, e2e_subprocess, e2e_tool_use, heartbeat, observability, onboard, providers, repl, sandbox_hardening, security, session, skills, subagent, telegram, voice.

## Quality gates

```
scripts/check-quality.sh       Work markers, lint bypasses, unsafe, ignored tests, file size limit (src/)
scripts/check-bdd-quality.sh   BDD anti-pattern detection (tests/bdd/)
cargo fmt --check              Formatting
cargo clippy -- -D warnings    Lints (zero warnings policy)
cargo test --lib               721 unit tests
cargo test --test architecture Clean Architecture boundary enforcement
cargo test --test bdd          BDD scenarios in `@done`/`@wip` features (`@real-llm` gated unless `QUECTO_REAL_LLM=1`)
```

Three-tier local hook model keeps pushes fast and gates expensive checks at merge time:

| Hook | Script | What it runs | When | ~Time |
|---|---|---|---|---|
| `pre-commit` | `scripts/pre-commit.sh` | quality scripts, fmt, clippy | every commit | ~20-40s |
| `pre-push` | `scripts/pre-push.sh` | unit tests + architecture + non-real BDD (25-way shards) | every push | ~30-60s |
| `pre-merge-commit` | `scripts/pre-merge-commit.sh` | real-LLM BDD (25-way shards), machete, deny | local merge to master | ~30-90s |

`scripts/pre-push.sh` runs quality + static checks, then a parallel test wave: `cargo test --lib`, `cargo test --test architecture`, and sharded non-real BDD via `scripts/run-bdd-shards.sh` (default `QUECTO_BDD_SHARDS=25`). Successful runs are cached per `HEAD` SHA + script hash in `.git/pre-push.passed.*`; use `QUECTO_PREPUSH_FORCE=1` to bypass cache. Logs are tee'd to `.git/pre-push.last.log`.

`scripts/pre-merge-commit.sh` runs merge-time checks: sharded real-LLM end-to-end tests (requires `OPENAI_API_KEY`, defaults `QUECTO_REAL_LLM_SHARDS=25`, tag via `QUECTO_REAL_LLM_TAG`), plus machete and deny. Cached per `HEAD` SHA + script hash in `.git/pre-merge-commit.passed.*`; use `QUECTO_PREMERGE_FORCE=1` to bypass cache. Logs are tee'd to `.git/pre-merge-commit.last.log`.

Coverage is no longer gated by hooks. Run coverage in nightly CI using `cargo llvm-cov` (recommended) to keep commit/push loops fast.

Run `scripts/install-hooks.sh` to install all three hooks. Set `QUECTO_TAG=real-llm-smoke` to run only the smoke subset of real-LLM tests.

### BDD quality gate (`scripts/check-bdd-quality.sh`)

Static analysis that blocks commits when step definitions violate BDD best practices. Steps should test application functions, not reimplement their own logic.

**Hard failures (block commit):**

| Check | What it catches |
|---|---|
| Tautological assertions | `assert!(true)`, `assert_eq!(x, x)` — tests that can never fail |
| Placeholder macros | `todo!()`, `unimplemented!()`, `panic!("not implemented")` in test code |
| TODO/FIXME/HACK/STUB comments | Unresolved work markers in step definitions |
| Discarded async results | `let _ = ...block_on(...)` — silently swallowed errors |
| Then steps with no assertions | Then steps whose body has no `assert!`/`unwrap()`/`expect()`/`panic!` |
| No-op When steps | When steps with empty bodies (comment-only stubs) |
| Silent error swallowing | `Err(_) => {}` — catch-all match arms that ignore errors |

**Warnings (non-blocking):**

| Check | What it catches |
|---|---|
| When steps that only assert | When steps that check preconditions instead of performing actions |
| Hand-rolled char parsing | `for ch in s.chars()` loops (should delegate to production code) |
| Manual JSON construction | `serde_json::Map::new()` (should use production serializers or helpers) |

## Tech stack

- Rust 2024 edition
- Tokio async runtime (rt-multi-thread, macros, signal, time, fs, process)
- reqwest with rustls-tls and stream feature (no OpenSSL dependency)
- serde/serde_json/serde_yaml for config, API payloads, and skill frontmatter
- uuid, chrono, tracing, dirs, thiserror
- Dev: cucumber 0.21, futures, tempfile, wiremock 0.6
