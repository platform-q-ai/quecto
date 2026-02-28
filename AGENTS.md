# Quecto

Rust reimplementation of an agentic AI assistant — single static binary targeting ultra-low resource usage (VPS, RPi, containers). Supports self-replication and bidirectional child process communication.

## Architecture

Four layers, strict dependency direction. Inner layers never import outer.

```
interface/ --> application/ --> domain/
                    |
infrastructure/ ----+
```

### domain/ — Pure types and traits
Zero deps except `thiserror`, `serde` (derive), `serde_yaml`. Defines system vocabulary.

| File | Purpose |
|---|---|
| `message.rs` | `Message` (constructors `::system/user/assistant/tool`; pruning fields: `turn`, `is_pinned`, `is_manifest`, `is_collapsed`, `tool_name`, `input_preview`, `spill_id`), `Role`, `ToolCall`, `LlmResponse`, `UsageInfo` |
| `provider.rs` | `LlmProvider` trait (dyn-compatible), `chat()` + `chat_stream()` (SSE with non-streaming fallback) |
| `tool.rs` | `Tool` trait, `ToolRegistry` trait, `ToolDefinition`, `ToolResult` |
| `agent.rs` | `AgentLoop` trait, `AgentInfo`, `AgentResult` |
| `session.rs` | `Session`, `SessionStore` trait, `SpillEntry`, `SpillIndex`, `ContextSpillStore` trait |
| `skill.rs` | `Skill`, `SkillSource`, `SkillFrontmatter`, `SkillLoader` trait, `split_skill_md()`, `validate_frontmatter()` |
| `cron.rs` | `CronJob`, `CronJobResult`, `CronSchedule`, `CronStore` trait |
| `channel.rs` | `Channel` trait (outbound delivery port) |
| `workspace.rs` | `HeartbeatTaskSource` and `OnboardStore` ports |
| `subagent.rs` | `SubagentConfig`, `validate_agent_id()` |
| `voice.rs` | `VoiceTranscriber` trait, `TranscriptionResult`, `TranscriptionError` |
| `error.rs` | `DomainError` enum (Provider, Tool, Session, Channel, Security, Config, Other) |

Traits use `Pin<Box<dyn Future + Send + '_>>` for `Arc<dyn Trait>` compatibility.

### application/ — Use cases
Depends only on `domain/`. Orchestration logic, no I/O.

| File | Purpose |
|---|---|
| `agent_loop.rs` | Core LLM-tool loop: send → execute tools → repeat. Traces `tool_name`, `duration_ms`, `is_error` |
| `context_pruning.rs` | Token estimation, 3-turn tool result collapse with spill-to-disk, sliding window, pinned manifest |
| `onboard.rs` | Onboarding orchestration via `OnboardStore` |
| `subagent.rs` | `SubagentContext` — child agent contexts with inherited sandbox |
| `cron_executor.rs` | Runs due cron jobs with timeout, records `last_error`, propagates `deliver_to` |
| `heartbeat.rs` | Task parsing, scheduling, dispatch through agent |
| `voice.rs` | Transcribes audio via `VoiceTranscriber`, routes text through agent |

### infrastructure/ — Concrete adapters
Implements domain traits with real I/O (serde, reqwest, tokio, filesystem).

| Component | Contents |
|---|---|
| `config.rs` | `Config` with serde, env overrides, exec isolation settings (nsjail binary/limits/fallback) |
| `providers/` | `OpenAiProvider`, `AnthropicProvider` (SSE streaming), `FallbackProvider` (cooldown + error classification). URL validation: https required for non-loopback |
| `tools/` | `ExecTool` (shell, 1MiB cap, native/nsjail modes), `ReadFile/WriteFile/EditFile/AppendFile/ListDir` (async tokio::fs), `SpawnTool`, `CronTool`, `MessageTool`, `WebSearchTool` (Brave+DDG), `RecallTool` (spill retrieval), `ToolRegistryImpl`, `tools/wasm/` (wasm32-wasip2 via Wasmtime Component Model) |
| `persistence/` | `FileSessionStore` (round-trips all Message fields), `MemoryStore`, `FileCronStore`, `FileSkillLoader`, `FileHeartbeatTaskSource`, `FileOnboardStore`, `FileContextSpillStore` (JSONL append-only) |
| `security/` | `Sandbox` — workspace path validation + command filtering |
| `auth/` | `CredentialStore` (file-based), `oauth.rs` (browser + device code flows) |
| `channels/` | `TelegramChannel` — send/receive, user allowlist, configurable `api_base` |
| `voice/` | `GroqWhisperClient` — Groq API speech-to-text |
| `logging.rs` | `redact_api_keys()` — pattern-based secret redaction |
| `bus.rs` | `MessageBus` — async channel for message passing |
| `health/` | `HealthServer` — raw tokio TCP, `/health` (liveness) + `/ready` (readiness) |

### Tool isolation

**WASM** (all tools except exec/spawn): Real wasm32-wasip2 via Wasmtime Component Model. Fresh `Store<HostState>` per invocation, fuel metering, memory limits, epoch interruption. Guest crate at `guest/` exports `quecto:tools/tool`.

**nsjail** (exec only): Linux process isolation via kernel namespaces + cgroups v2. Workspace RW, toolchain RO, memory/PID/CPU limits, Kafel seccomp-bpf. Configure via `tools.exec.isolation`, `tools.exec.nsjail_binary`, `tools.exec.allow_native_fallback`.

### interface/ — CLI + Gateway (composition root)
Manual arg parsing (no clap). Entry point: `cli::run(args) -> i32`.

| Command | Description |
|---|---|
| `quecto` | Interactive REPL (`-s` session, `--system` prompt, `--model` override) |
| `quecto agent -m <msg>` | Headless one-shot (`-s`, `--system`, `--model`, `--max-iterations`, `--max-time`) |
| `quecto onboard` | Creates workspace + default config |
| `quecto skills list\|remove\|install` | Skill management |
| `quecto status` | Config summary, provider availability |
| `quecto auth login\|logout\|status` | Credential management (token/OAuth/device-code) |
| `quecto gateway` | Full async gateway (Telegram polling + agent loop) |
| `quecto help\|version` | Self-explanatory |

REPL commands: `/help`, `/clear`, `/cron`, `/heartbeat`, `/agent`, `/spawn`, `/exit`. Uses abstracted I/O for testing.

Gateway: `EventLoopContext` holds runtime state. Telegram polling, bot commands (`/start`, `/help`, `/status`), credential snapshot for efficiency, session trimming via `max_session_messages`, graceful shutdown via `tokio::select!`.

## Dependency rule
- `domain/` imports nothing from project
- `application/` imports `domain/` only
- `infrastructure/` imports `domain/` only (implements traits)
- `interface/` imports all three (composition root)

## Development workflow

BDD-first with cucumber-rs: `@pending` → `@wip` → write steps → implement → `@done`. Runner uses `.fail_on_skipped()`, runs `@wip`+`@done`. `@real-llm` gated by `QUECTO_REAL_LLM=1`. `QUECTO_TAG=<tag>` for filtering. 49 feature files, 25 step definition modules.

## Quality gates

| Gate | Command |
|---|---|
| Quality scripts | `scripts/check-quality.sh`, `scripts/check-bdd-quality.sh` |
| Format | `cargo fmt --check` |
| Lint | `cargo clippy -- -D warnings` (zero warnings) |
| Unit tests | `cargo test --lib` (831 tests) |
| Architecture | `cargo test --test architecture` |
| BDD | `cargo test --test bdd` |

Three-tier hooks: pre-commit (~20-40s: quality+fmt+clippy), pre-push (~30-60s: tests+25-shard BDD), pre-merge (~30-90s: real-LLM+machete+deny). SHA-based caching. Install via `scripts/install-hooks.sh`.

### Sharded BDD (25-way parallel)

Non-real-LLM (fast, no API key needed):
```bash
for i in $(seq 0 24); do
  (timeout 12m env QUECTO_BDD_SHARD_INDEX=$i QUECTO_BDD_SHARD_TOTAL=25 cargo test --test bdd) &
done
wait
```

Real-LLM (requires `OPENAI_API_KEY`):
```bash
for i in $(seq 0 24); do
  (timeout 12m env QUECTO_REAL_LLM=1 QUECTO_TAG=real-llm QUECTO_BDD_SHARD_INDEX=$i QUECTO_BDD_SHARD_TOTAL=25 cargo test --test bdd) &
done
wait
```

Use `QUECTO_TAG=real-llm-smoke` for quicker paid smoke runs.

### Running individual features or scenarios

To debug a single scenario, add a temporary tag (e.g. `@focus`) to the scenario in the `.feature` file, then run:
```bash
QUECTO_TAG=focus cargo test --test bdd
```
Remove the tag before committing.

## Tech stack
Rust 2024, Tokio, reqwest+rustls, serde/serde_json/serde_yaml, uuid, chrono, tracing, dirs, thiserror. Dev: cucumber 0.21, futures, tempfile, wiremock 0.6.
