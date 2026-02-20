# Quecto: BDD Implementation Plan

## Dev Cycle (per feature)

```
1.  Change @pending -> @wip on the feature file
2.  cargo test --test bdd             -> FAIL (skipped steps)
3.  Write step definitions            -> FAIL (logic missing)
4.  Write unit tests                  -> FAIL (red)
5.  Implement production code
6.  cargo test --lib                  -> PASS (green)
7.  cargo test --test bdd             -> PASS (green)
8.  Refactor
9.  Change @wip -> @done
```

## Implementation Order

Features are ordered by dependency — each feature builds on the ones above it.

---

### 1. config (4 scenarios)

**Why first:** Every other feature depends on loading config.

**Steps:**
- Write step defs: load JSON from temp file, assert field values, env override, tilde expansion
- Unit tests: Config deserialization, defaults, env var override, workspace_path expansion
- Implement: `infrastructure/config.rs` — add `Config::load(path)`, `Config::load_with_env()`, file I/O
- Files touched: `infrastructure/config.rs`

---

### 2. cli (7 scenarios)

**Why second:** The CLI dispatches all commands; needed to test everything else end-to-end.

**Steps:**
- Write step defs: capture stdout/stderr/exit_code from `cli::run()`, regex matching
- Unit tests: arg parsing, help output, version output, unknown command error
- Implement: `interface/cli.rs` — already stubbed, wire up real dispatch to config loading
- Files touched: `interface/cli.rs`
- Depends on: config (for `agent -m` scenario)

---

### 3. onboard (4 scenarios)

**Why third:** Creates the workspace and config that all runtime features need.

**Steps:**
- Write step defs: temp dir setup, assert file/dir existence, assert config defaults
- Unit tests: workspace creation, template file writing, existing config detection
- Implement: `application/onboard.rs` — create config.json, workspace dir, template files (AGENTS.md, IDENTITY.md, SOUL.md, TOOLS.md, USER.md)
- Files touched: `application/onboard.rs`, `interface/cli.rs` (wire onboard command)

---

### 4. security (8 scenarios + 6 outline examples)

**Why fourth:** Tools depend on the sandbox; implement it before any tools.

**Steps:**
- Write step defs: sandbox with temp workspace, assert errors for blocked paths/commands
- Unit tests: path validation (inside/outside workspace), dangerous command regex matching, path traversal prevention
- Implement: `infrastructure/security/sandbox.rs` — `Sandbox::validate_path()`, `Sandbox::validate_command()`, dangerous pattern list
- Files touched: `infrastructure/security/sandbox.rs`

---

### 5. agent_tools (10 scenarios)

**Why fifth:** Tools are needed by the agent loop, security sandbox is in place.

**Steps:**
- Write step defs: create temp workspace, execute tools, assert results and file contents
- Unit tests: each tool (exec, read_file, write_file, edit_file, append_file, list_dir) with sandbox, registry lookup
- Implement:
  - `infrastructure/tools/exec.rs` — shell exec with sandbox check
  - `infrastructure/tools/filesystem.rs` — read/write/edit/append/list with sandbox
  - `infrastructure/tools/web_search.rs` — DuckDuckGo HTTP client (mock in tests)
  - `infrastructure/tools/message.rs` — send via bus
  - `infrastructure/tools/registry.rs` — register tools, lookup by name
- Files touched: `infrastructure/tools/*.rs`, `domain/tool.rs` (may need ToolRegistry trait)

---

### 6. providers (8 scenarios)

**Why sixth:** The agent loop needs an LLM provider to function.

**Steps:**
- Write step defs: mock HTTP server (wiremock), assert request/response format, fallback behavior
- Unit tests: OpenAI request serialization, Anthropic request serialization, response parsing, error classification, cooldown timer, fallback logic
- Implement:
  - `infrastructure/providers/openai.rs` — `impl LlmProvider`, HTTP request/response mapping
  - `infrastructure/providers/anthropic.rs` — `impl LlmProvider`, HTTP request/response mapping
  - `infrastructure/providers/fallback.rs` — `FallbackProvider` wrapping multiple providers, cooldown, error classifier
- Files touched: `infrastructure/providers/*.rs`

---

### 7. agent_loop (6 scenarios)

**Why seventh:** Core orchestration — requires tools + providers to be in place.

**Steps:**
- Write step defs: mock LLM returning text/tool_calls, assert tool execution order, iteration limit
- Unit tests: message processing loop, tool call parsing, iteration counting, system prompt assembly
- Implement: `application/agent_loop.rs` — `AgentLoopImpl` with LLM-tool loop, max iterations, tool definition injection
- Files touched: `application/agent_loop.rs`
- Depends on: providers, agent_tools

---

### 8. session (8 scenarios)

**Why eighth:** Agent loop needs session to persist conversation history.

**Steps:**
- Write step defs: create/load/save sessions in temp workspace, assert routing keys
- Unit tests: session key generation, load/save round-trip, memory file write, identity loading
- Implement:
  - `infrastructure/persistence/session_store.rs` — `impl SessionStore`, JSON file per session
  - `infrastructure/persistence/memory_store.rs` — read/write MEMORY.md
- Files touched: `infrastructure/persistence/session_store.rs`, `infrastructure/persistence/memory_store.rs`

---

### 9. auth (8 scenarios)

**Why ninth:** Needed before gateway can authenticate with providers.

**Steps:**
- Write step defs: mock credential store, assert store/load/delete, auth status output
- Unit tests: credential serialization, expiry checking, store/load/delete operations
- Implement:
  - `infrastructure/auth/credential_store.rs` — file-based encrypted credential storage
  - `infrastructure/auth/oauth.rs` — OAuth device-code flow (mock in tests)
  - `interface/cli.rs` — wire auth login/logout/status subcommands
- Files touched: `infrastructure/auth/*.rs`, `interface/cli.rs`

---

### 10. telegram (8 scenarios)

**Why tenth:** Gateway channel — requires agent loop, session, security to be in place.

**Steps:**
- Write step defs: mock Telegram API, simulate incoming messages, assert routing/rejection
- Unit tests: message parsing, allow_from filtering, bot command handling, graceful shutdown
- Implement:
  - `infrastructure/channels/telegram.rs` — `impl Channel`, long-polling, message routing
  - `infrastructure/bus.rs` — async inbound/outbound message channels
- Files touched: `infrastructure/channels/telegram.rs`, `infrastructure/bus.rs`
- Depends on: agent_loop, session, security

---

### 11. cron (10 scenarios)

**Why eleventh:** Scheduled tasks — requires agent loop and persistence.

**Steps:**
- Write step defs: add/list/remove/enable/disable jobs via CLI, assert persistence and execution
- Unit tests: job serialization, schedule parsing (interval + cron expression), timeout enforcement
- Implement:
  - `infrastructure/persistence/cron_store.rs` — `impl CronStore`, JSON file storage
  - `infrastructure/tools/cron_tool.rs` — agent-facing cron tool
  - `interface/cli.rs` — wire cron add/list/remove/enable/disable subcommands
- Files touched: `infrastructure/persistence/cron_store.rs`, `infrastructure/tools/cron_tool.rs`, `interface/cli.rs`

---

### 12. subagent (6 scenarios)

**Why twelfth:** Depends on agent loop, tools, security, session.

**Steps:**
- Write step defs: spawn subagent, assert independent context, message delivery, allowlist
- Unit tests: subagent context isolation, workspace restriction inheritance, agent_id validation
- Implement:
  - `application/subagent.rs` — spawn logic, context isolation, allowlist checking
  - `infrastructure/tools/spawn.rs` — `impl Tool` for spawn
- Files touched: `application/subagent.rs`, `infrastructure/tools/spawn.rs`

---

### 13. heartbeat (6 scenarios)

**Why thirteenth:** Depends on agent loop, subagent, workspace reading.

**Steps:**
- Write step defs: create HEARTBEAT.md in temp workspace, trigger heartbeat, assert task execution
- Unit tests: markdown parsing, interval timing, missing file handling, subagent spawning
- Implement: `application/heartbeat.rs` — read HEARTBEAT.md, parse tasks, dispatch to agent/subagent
- Files touched: `application/heartbeat.rs`
- Depends on: agent_loop, subagent

---

### 14. skills (8 scenarios)

**Why fourteenth:** Depends on workspace, config, agent initialization.

**Steps:**
- Write step defs: create skill dirs in temp workspace, CLI list/install/remove, assert system prompt
- Unit tests: skill loading from workspace/global/builtin, SKILL.md parsing, system prompt injection
- Implement:
  - `infrastructure/persistence/skill_loader.rs` — `impl SkillLoader`, multi-source resolution
  - `interface/cli.rs` — wire skills list/install/remove/show/search subcommands
- Files touched: `infrastructure/persistence/skill_loader.rs`, `interface/cli.rs`

---

### 15. voice (3 scenarios)

**Why fifteenth:** Depends on Telegram channel being in place.

**Steps:**
- Write step defs: mock Groq Whisper API, simulate voice message, assert transcription routing
- Unit tests: audio file download, Whisper API request/response, error handling
- Implement: `infrastructure/voice/groq_whisper.rs` — Groq Whisper HTTP client, audio download from Telegram
- Files touched: `infrastructure/voice/groq_whisper.rs`
- Depends on: telegram

---

### 16. observability (6 scenarios)

**Why last:** Cross-cutting concern, depends on gateway, config, tools all being in place.

**Steps:**
- Write step defs: start gateway, HTTP requests to /health and /ready, assert status output, log capture
- Unit tests: health endpoint responses, status command output formatting, API key redaction in logs
- Implement:
  - `infrastructure/health/server.rs` — HTTP server with /health and /ready
  - `interface/cli.rs` — wire status command with config/provider summary
  - Logging: configure tracing-subscriber with key redaction
- Files touched: `infrastructure/health/server.rs`, `interface/cli.rs`

---

## Summary

| # | Feature | Scenarios | Key files | Depends on |
|---|---|---|---|---|
| 1 | config | 4 | infrastructure/config.rs | — |
| 2 | cli | 7 | interface/cli.rs | config |
| 3 | onboard | 4 | application/onboard.rs | config, cli |
| 4 | security | 14 | infrastructure/security/sandbox.rs | — |
| 5 | agent_tools | 10 | infrastructure/tools/*.rs | security |
| 6 | providers | 8 | infrastructure/providers/*.rs | — |
| 7 | agent_loop | 6 | application/agent_loop.rs | providers, agent_tools |
| 8 | session | 8 | infrastructure/persistence/session_store.rs | — |
| 9 | auth | 8 | infrastructure/auth/*.rs | config, cli |
| 10 | telegram | 8 | infrastructure/channels/telegram.rs | agent_loop, session, security |
| 11 | cron | 10 | infrastructure/persistence/cron_store.rs | agent_loop, cli |
| 12 | subagent | 6 | application/subagent.rs | agent_loop, security |
| 13 | heartbeat | 6 | application/heartbeat.rs | agent_loop, subagent |
| 14 | skills | 8 | infrastructure/persistence/skill_loader.rs | config, cli |
| 15 | voice | 3 | infrastructure/voice/groq_whisper.rs | telegram |
| 16 | observability | 6 | infrastructure/health/server.rs | gateway, config |
| | **Total** | **116** | | |
