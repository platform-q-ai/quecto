# Quecto

Rust reimplementation of an agentic AI assistant — single static binary targeting ultra-low resource usage (VPS, RPi, containers). Supports self-replication and bidirectional child process communication.

**ALWAYS FOLLOW BDD/TDD RED, GREEN, REFACTOR PROCESS WHEN MAKING CHANGES**
**ALWAYS USE FULL DEVELOPMENT WORKFLOW**

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
| `provider.rs` | `LlmProvider` trait (dyn-compatible), `ChatRequest` (with `session_id` for prompt caching), `chat()` + `chat_stream()` (SSE with non-streaming fallback), `model_excluded_from_provider()` (routes `claude-*` → Anthropic) |
| `tool.rs` | `Tool` trait, `ToolRegistry` trait, `ToolDefinition`, `ToolResult` (with `image_blocks` for base64 images) |
| `agent.rs` | `AgentLoop` trait, `AgentInfo`, `AgentResult`, `AgentProgressEvent`, `ProgressCallback` |
| `session.rs` | `Session`, `SessionStore` trait, `SpillEntry`, `SpillIndex`, `ContextSpillStore` trait, `strip_tool_history()` |
| `skill.rs` | `Skill`, `SkillSource`, `SkillFrontmatter`, `SkillLoader` trait, `split_skill_md()`, `validate_frontmatter()` |
| `cron.rs` | `CronJob`, `CronJobResult`, `CronSchedule`, `CronStore` trait, `is_job_due()` (saturating arithmetic) |
| `channel.rs` | `Channel` trait (outbound delivery port) |
| `workspace.rs` | `HeartbeatTaskSource` and `OnboardStore` ports |
| `subagent.rs` | `SubagentConfig`, `validate_agent_id()` |
| `voice.rs` | `VoiceTranscriber` trait, `TranscriptionResult`, `TranscriptionError` |
| `workflow.rs` | `WorkflowState`, `WorkflowConfig` (`guard_commit`, `enforce_commit_after_step`, `steps`), `WorkflowStep`, `WorkflowPersistable`, `default_steps()` (returns empty — steps must be configured in config.json), `bdd_steps()` (test-only 16-step template), `from_persistable_with_steps()` |
| `error.rs` | `DomainError` enum (Provider, Tool, Session, Channel, Security, Config, Other) |

Traits use `Pin<Box<dyn Future + Send + '_>>` for `Arc<dyn Trait>` compatibility.

### application/ — Use cases
Depends only on `domain/`. Orchestration logic, no I/O.

| File | Purpose |
|---|---|
| `agent_loop.rs` | Core LLM-tool loop: send → execute tools → repeat. Traces `tool_name`, `duration_ms`, `is_error`. Progress callbacks for REPL spinner |
| `context_pruning.rs` | Token estimation, sliding window, pinned manifest. Collapse disabled by default (`context_collapse_after_turns = u32::MAX`); spill-to-disk when enabled |
| `cron_executor.rs` | Runs due cron jobs with timeout, records `last_error`, propagates `deliver_to` |
| `heartbeat.rs` | Task parsing, scheduling, dispatch through agent |
| `onboard.rs` | Onboarding orchestration via `OnboardStore` |
| `reload.rs` | `/reload` use case: strips stale tool history via `strip_tool_history()`, clears spill index, coordinates `SessionStore` + `ContextSpillStore` |
| `subagent.rs` | `SubagentContext` — child agent contexts with inherited sandbox |
| `voice.rs` | Transcribes audio via `VoiceTranscriber`, routes text through agent |

### infrastructure/ — Concrete adapters
Implements domain traits with real I/O (serde, reqwest, tokio, filesystem).

| Component | Contents |
|---|---|
| `config.rs` | `Config` with serde, env overrides, exec isolation settings (nsjail binary/limits/fallback), `TelegramConfig.default_send_to`, `WorkflowConfig` (steps must be explicit in config.json, `guard_commit` controls WorkflowGuard registration) |
| `providers/` | `OpenAiProvider`, `AnthropicProvider` (SSE streaming), `CodexProvider` (Responses API, SSE, `prompt_cache_key`, orphan pair repair), `FallbackProvider` (cooldown + error classification + `claude-*` model routing). URL validation: https required for non-loopback |
| `tools/` | `bash/` (shell, 1MiB cap, per-invocation timeout, `commandPrefix`, native/nsjail modes), `filesystem/` (`ReadTool` with image base64+auto-resize, `WriteTool`, `EditTool` with fuzzy match+CRLF/BOM+LCS diff, `LsTool` with limit+case-insensitive sort), `grep.rs` (rg JSON output, file-cache context), `find.rs` (fd, nested .gitignore, path-segment globs via `--full-path`), `ensure_tool.rs` (auto-download rg/fd from GitHub), `spawn.rs`, `cron_tool.rs`, `message.rs` (with `default_send_to` fallback), `web_search.rs` (Brave+DDG), `recall.rs` (spill retrieval), `workflow_tool.rs` (`WorkflowTool` + `WorkflowGuard` — blocks `git commit`/`git push` when workflow steps incomplete; registration controlled by `guard_commit` config), `path_utils.rs`, `truncate.rs`, `registry.rs` (`ToolRegistryImpl`, `guard_count()`) |
| `persistence/` | `FileSessionStore` (round-trips all Message fields), `MemoryStore`, `FileCronStore` (Mutex-serialized read-modify-write, atomic temp-file rename), `FileSkillLoader`, `FileHeartbeatTaskSource`, `FileOnboardStore`, `FileContextSpillStore` (JSONL append-only) |
| `security/` | `Sandbox` — workspace path validation + command filtering |
| `auth/` | `CredentialStore` (file-based), `oauth.rs` (browser + device code flows, Anthropic OAuth) |
| `channels/` | `TelegramChannel` — send/receive, user allowlist, configurable `api_base`, `default_send_to` |
| `voice/` | `GroqWhisperClient` — Groq API speech-to-text |
| `logging.rs` | `redact_api_keys()` — pattern-based secret redaction |
| `bus.rs` | `MessageBus` — async channel for message passing |
| `health/` | `HealthServer` — raw tokio TCP, `/health` (liveness) + `/ready` (readiness) |

### Tool isolation

**Filesystem tools** (`read`, `write`, `edit`, `ls`): `Sandbox::validate_path` — canonicalises the path, follows symlinks at every component, and rejects anything outside `canonical_workspace`. Called before any I/O.

**bash** (exec only): nsjail for process isolation via Linux kernel namespaces + rlimits. Workspace RW, toolchain RO, memory/PID/CPU limits via `--rlimit_as`/`--rlimit_nproc`/`--rlimit_cpu` (no cgroup access required). Defaults: 4 GB AS, 256 PIDs, no timeout (configure CPU/wall limits via config), 512 MB tmpfs. Configure via `tools.exec.isolation`, `tools.exec.nsjail_binary`, `tools.exec.allow_native_fallback`.

**Tool binary resolution** (`rg`, `fd`): `ensure_tool` resolves via system PATH → cache dir (`~/.local/share/quecto/tools/`) → auto-download from GitHub releases. Set `QUECTO_OFFLINE=1` to disable downloads.

### interface/ — CLI + Gateway (composition root)
Manual arg parsing (no clap). Entry point: `cli::run(args) -> i32`.

| Command | Description |
|---|---|
| `quecto` | Interactive REPL (`-s` session, `--system` prompt, `--model` override) with live progress spinner |
| `quecto agent -m <msg>` | Headless one-shot / automation (`-s`, `--no-session`, `--system`, `--model`, `--max-iterations`, `--max-time`, `--mode uds`) |
| `quecto onboard` | Creates workspace + default config |
| `quecto skills list\|remove\|install` | Skill management |
| `quecto status` | Config summary, provider availability |
| `quecto auth login\|logout\|status` | Credential management (token/OAuth/device-code) |
| `quecto gateway` | Full async gateway (Telegram polling + agent loop) |
| `quecto help\|version` | Self-explanatory |

REPL commands: `/help`, `/clear`, `/cron`, `/heartbeat`, `/agent`, `/spawn`, `/exit`. Uses abstracted I/O for testing.

REPL progress: `ProgressRenderer` drives a braille spinner at ~12fps on stderr (TTY only). Shows thinking state, tool name, arguments preview, and execution status. Pure ANSI escape codes — no external crates.

All entry points (REPL, CLI agent, gateway) prepend a datetime preamble to the system prompt via `build_system_prompt()` so the agent always knows the current date/time/timezone — critical for cron scheduling and time-aware tasks.

Gateway: `EventLoopContext` holds runtime state. Telegram polling, bot commands (`/start`, `/help`, `/status`, `/reload`), credential snapshot for efficiency, session trimming via `max_session_messages`, graceful shutdown via `tokio::select!`. Gateway services use a `SystemPromptAgent` wrapper that injects a transient datetime+skills system prompt before each `process()` call and strips it after — never persisted in session history. Applied to inbound message processing, heartbeat ticks, and cron ticks.

Headless CLI agent includes `SpawnTool` for background subagent spawning. Subagent timeout: 24 hours.

## Dependency rule
- `domain/` imports nothing from project
- `application/` imports `domain/` only
- `infrastructure/` imports `domain/` only (implements traits)
- `interface/` imports all three (composition root)

## Development workflow

1 - Update Scenarios/Add new features as necessary
2 - Write/update unit tests
3 - ensure new/modified tests fail -RED
4 - Implement code - GREEN
5 - Refactor based in performance, security and clean architecture standards - REFACTOR
6 - Ensure tests still pass - GREEN
7 - Commit
8 - Push
9 - Create PR
10 - Despatch Architecture, Security, Performance Reviewers
11 - Fix all valid concerns raised in review comments
12 - Push changes to remote
13 - Reply to comments and mark resolved
14 - Merge
15 - Move to local master and pull

## Quality gates

| Gate | Command |
|---|---|
| Quality scripts | `scripts/check-quality.sh`, `scripts/check-bdd-quality.sh` |
| Format | `cargo fmt --check` |
| Lint | `cargo clippy -- -D warnings` (zero warnings) |
| Unit tests | `cargo test --no-fail-fast --lib 2>&1 \| scripts/test-filter.sh` |
| Architecture | `cargo test --no-fail-fast --test architecture 2>&1 \| scripts/test-filter.sh` |
| BDD (sharded) | See [Sharded BDD](#sharded-bdd-24-way-parallel) below |

All test commands pipe through `scripts/test-filter.sh` which strips the per-test `... ok` noise and shows only:
- **Summary totals** (passed/failed counts)
- **Failure details** (test name, file:line, assertion message, panic reason)
- **BDD failures** with Feature/Scenario context

`--no-fail-fast` ensures all failures are reported in a single run, not just the first.

Three-tier hooks: pre-commit (~20-40s: quality+fmt+clippy), pre-push (~30-60s: tests+24-shard BDD), pre-merge (~30-90s: real-LLM+machete+deny). SHA-based caching. Install via `scripts/install-hooks.sh`.

### Sharded BDD (24-way parallel)

Non-real-LLM (fast, no API key needed):
```bash
(for i in $(seq 0 23); do
  (timeout 12m env QUECTO_BDD_SHARD_INDEX=$i QUECTO_BDD_SHARD_TOTAL=24 cargo test --no-fail-fast --features test-support --test bdd 2>&1 | scripts/test-filter.sh) &
done
wait)
```

Real-LLM (requires `OPENAI_API_KEY`):
```bash
(for i in $(seq 0 23); do
  (timeout 12m env QUECTO_REAL_LLM=1 QUECTO_TAG=real-llm QUECTO_BDD_SHARD_INDEX=$i QUECTO_BDD_SHARD_TOTAL=24 cargo test --no-fail-fast --features test-support --test bdd 2>&1 | scripts/test-filter.sh) &
done
wait)
```

Use `QUECTO_TAG=real-llm-smoke` for quicker paid smoke runs.

### Running individual features or scenarios

To debug a single scenario, add a temporary tag (e.g. `@focus`) to the scenario in the `.feature` file, then run:
```bash
QUECTO_TAG=focus cargo test --no-fail-fast --features test-support --test bdd 2>&1 | scripts/test-filter.sh
```
Remove the tag before committing.

## Tech stack
Rust 2024, Tokio, reqwest+rustls, serde/serde_json/serde_yaml, uuid, chrono, tracing, dirs, thiserror, similar, base64, sha2, image, flate2, tar. Dev: cucumber 0.21, futures, tempfile, wiremock 0.6.

**ALWAYS FOLLOW BDD/TDD RED, GREEN, REFACTOR PROCESS WHEN MAKING CHANGES**
**ALWAYS USE FULL DEVELOPMENT WORKFLOW**
