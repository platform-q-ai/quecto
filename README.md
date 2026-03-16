# Quecto

A single-binary personal AI assistant that runs on minimal Linux systems. Quecto receives messages via the command line or a UDS event bus, routes them through an LLM (OpenAI, Anthropic, or ChatGPT Codex), executes tools (shell commands, file operations, search), and persists conversations to disk.

Built in Rust. No runtime dependencies. Runs on a VPS, Raspberry Pi, or container.

**ALWAYS FOLLOW BDD/TDD RED, GREEN, REFACTOR PROCESS WHEN MAKING CHANGES**
**ALWAYS USE FULL DEVELOPMENT WORKFLOW**

## Release Notes

Current version: **0.22.0** — see [`CHANGELOG.md`](CHANGELOG.md) for full history.

## Quick Start

```bash
# Install
cargo install --path .

# Set up config and workspace
quecto onboard

# Store your API key
quecto auth login --provider openai --token sk-proj-your-key

# Talk to the agent (one-shot)
quecto agent -m "Hello, what can you do?"

# Or start an interactive session
quecto
```

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
| `message.rs` | `Message` (constructors `::system/user/assistant/tool`; pruning fields: `turn`, `is_pinned`, `is_manifest`, `is_collapsed`, `tool_name`, `input_preview`, `spill_id`; `image_blocks` for tool results, `user_image_blocks` for user images, `is_error`, `stop_reason`), `Role`, `ToolCall`, `LlmResponse`, `UsageInfo`, `StopReason` (maps `model_context_window_exceeded` → `MaxTokens`), `UserImageBlock` |
| `provider.rs` | `LlmProvider` trait (dyn-compatible), `ChatRequest` (with `session_id` for prompt caching, `cancel_flag`, `thinking_level`, `tool_choice`, `metadata`, `effort`), `chat()` + `chat_stream()` (SSE with non-streaming fallback) + `chat_stream_incremental()` (real-time `StreamEvent` channel), `CancelFlag`, `ThinkingLevel`, `ToolChoice`, `RequestMetadata`, `EffortLevel` (with `parse()` / `as_str()`) |
| `tool.rs` | `Tool` trait, `ToolRegistry` trait, `ToolGuard` trait, `ToolDefinition` (with `Cow<'static, str>` fields), `ToolResult` (with `image_blocks` for base64 images), `ImageBlock` |
| `agent.rs` | `AgentLoop` trait, `AgentInfo`, `AgentResult`, `AgentProgressEvent` (with `tool_call_id` on `ToolStarted`/`ToolFinished`, `Token` for streaming, `Thinking` with context stats), `ProgressCallback` |
| `session.rs` | `Session`, `SessionStore` trait, `SpillEntry`, `SpillIndex`, `ContextSpillStore` trait, `strip_tool_history()`, `filter_orphan_tool_pairs()` (with `OrphanDiag`) |
| `skill.rs` | `Skill`, `SkillSource`, `SkillFrontmatter`, `SkillLoader` trait, `split_skill_md()`, `validate_frontmatter()`, `is_valid_skill_name()` |
| `extension.rs` | `Extension` trait (`name()`, `tools()`, `system_prompt_snippet()`) |
| `workspace.rs` | `OnboardStore` port |
| `subagent.rs` | `SubagentConfig`, `validate_agent_id()` |
| `workflow.rs` | `WorkflowState`, `WorkflowConfig` (`guard_commit`, `enforce_commit_after_step`, `steps`), `WorkflowStep`, `WorkflowPersistable`, `WorkflowProgress`, `WorkflowError`, `default_steps()` (returns empty — steps must be configured in config.json), `bdd_steps()` (test-only 16-step template), `from_persistable_with_steps()` |
| `error.rs` | `DomainError` enum (Provider, Tool, Session, Security, Config, Other) |

Traits use `Pin<Box<dyn Future + Send + '_>>` for `Arc<dyn Trait>` compatibility.

### application/ — Use cases
Depends only on `domain/`. Orchestration logic, no I/O.

| File | Purpose |
|---|---|
| `agent_loop.rs` | Core LLM-tool loop: send → execute tools → repeat. Traces `tool_name`, `duration_ms`, `is_error`. Progress callbacks for REPL spinner. Supports incremental streaming via `chat_stream_incremental()`. Passes configured `effort` level through to every `ChatRequest` |
| `context_pruning.rs` | Token estimation, sliding window, pinned manifest. Collapse disabled by default (`context_collapse_after_turns = u32::MAX`); spill-to-disk when enabled |
| `onboard.rs` | Onboarding orchestration via `OnboardStore` |
| `reload.rs` | `/reload` use case: strips stale tool history via `strip_tool_history()`, clears spill index, coordinates `SessionStore` + `ContextSpillStore` |
| `subagent.rs` | `SubagentContext` — child agent contexts with inherited sandbox |

### infrastructure/ — Concrete adapters
Implements domain traits with real I/O (serde, reqwest, tokio, filesystem).

| Component | Contents |
|---|---|
| `config.rs` | `Config` with serde, env overrides (`QUECTO_AGENTS_DEFAULTS_EFFORT` validated at load), exec isolation settings (nsjail binary/limits/fallback), `WorkflowConfig` (steps must be explicit in config.json, `guard_commit` controls WorkflowGuard registration). Tolerates unknown fields (forward-compatible) |
| `providers/` | `OpenAiProvider` (SSE streaming via `openai_sse`), `AnthropicProvider` (SSE streaming via `anthropic_sse`, extended thinking support, auto-enables adaptive thinking for 4.6 models, effort default `low` for 4.6 models), `CodexProvider` (Responses API, SSE, `prompt_cache_key`, orphan pair repair), `RefreshableProvider` (OAuth 401 → auto-refresh → retry), `FallbackProvider` (cooldown + error classification + `provider/model` routing syntax). URL validation: https required for non-loopback (override with `QUECTO_ALLOW_CUSTOM_PROVIDER_HOSTS=1`) |
| `tools/` | `bash/` (shell, 1MiB cap, per-invocation timeout, `commandPrefix`, native/nsjail modes), `filesystem/` (`ReadTool` with image base64+auto-resize, `WriteTool`, `EditTool` with fuzzy match+CRLF/BOM+LCS diff, `LsTool` with limit+case-insensitive sort), `grep.rs` (rg JSON output, file-cache context), `find.rs` (fd, nested .gitignore, path-segment globs via `--full-path`), `ensure_tool.rs` (auto-download rg/fd from GitHub), `spawn.rs` (background UDS-mode subagent spawning), `agent_cmd.rs` (send commands to spawned UDS agents — `steer`, `follow_up`, `abort`, `get_state`), `web_search.rs` (Brave+DDG), `web_fetch.rs` (URL fetch with HTML stripping, per-host SSRF allowlist for tests), `recall.rs` (spill retrieval), `workflow_tool.rs` (`WorkflowTool` + `WorkflowGuard` — blocks `git commit`/`git push` when workflow steps incomplete; registration controlled by `guard_commit` config), `path_utils.rs`, `truncate.rs`, `command_match.rs`, `registry.rs` (`ToolRegistryImpl`, `guard_count()`) |
| `persistence/` | `FileSessionStore` (round-trips all Message fields), `MemoryStore`, `FileSkillLoader`, `FileOnboardStore` (`workspace_store.rs`), `FileContextSpillStore` (JSONL append-only) |
| `security/` | `Sandbox` — workspace path validation + command filtering |
| `extensions/` | `ExtensionRegistry` (register extensions, aggregate tools + system prompt snippets), `NativeExtension` (compiled-in config-gated tools, e.g. `web_search`, `web_fetch`), `UdsExtensionTool` (routes tool execution to connected UDS clients via mpsc/oneshot channels). See [Extensions guide](docs/extensions.md) |
| `auth/` | `CredentialStore` (file-based, `AuthMethod::Token`/`OAuth`), `oauth.rs` (browser + device code flows, Anthropic OAuth, OpenAI account ID extraction from JWT) |
| `logging.rs` | `redact_api_keys()` — pattern-based secret redaction |

### Tool isolation

**Filesystem tools** (`read`, `write`, `edit`, `ls`): `Sandbox::validate_path` — canonicalises the path, follows symlinks at every component, and rejects anything outside `canonical_workspace`. Called before any I/O.

**bash** (exec only): nsjail for process isolation via Linux kernel namespaces + rlimits. Workspace RW, toolchain RO, memory/PID/CPU limits via `--rlimit_as`/`--rlimit_nproc`/`--rlimit_cpu` (no cgroup access required). Defaults: 4 GB AS, 256 PIDs, no timeout (configure CPU/wall limits via config), 512 MB tmpfs. Configure via `tools.exec.isolation`, `tools.exec.nsjail_binary`, `tools.exec.allow_native_fallback`. Network namespace isolation disabled with `--network` flag or `tools.exec.network_passthrough` config.

**Tool binary resolution** (`rg`, `fd`): `ensure_tool` resolves via system PATH → cache dir (`~/.local/share/quecto/tools/`) → auto-download from GitHub releases. Set `QUECTO_OFFLINE=1` to disable downloads.

### interface/ — CLI (composition root)
Manual arg parsing (no clap). Entry point: `cli::run(args) -> i32`.

| Command | Description |
|---|---|
| `quecto` | Interactive REPL (`-s` session, `--system` prompt, `--model` override, `--no-sandbox`, `--network`) with live progress spinner |
| `quecto agent -m <msg>` | Headless one-shot (`-s`, `--no-session`, `--system`, `--model`, `--max-iterations`, `--max-time`, `--no-sandbox`, `--network`) |
| `quecto agent --mode uds` | Persistent UDS event bus: multi-client JSON-lines protocol over Unix domain socket (`--socket <path>` for explicit path, auto-generated otherwise) |
| `quecto onboard` | Creates workspace + default config |
| `quecto skills list\|remove\|install` | Skill management |
| `quecto status` | Config summary, provider availability |
| `quecto auth login\|logout\|status` | Credential management (token/OAuth/device-code) |
| `quecto help\|version` | Self-explanatory |

REPL commands: `/help`, `/clear`, `/agent`, `/spawn`, `/exit`, `/quit`. Uses abstracted I/O for testing.

REPL progress: `ProgressRenderer` drives a braille spinner at ~12fps on stderr (TTY only). Shows thinking state, tool name, arguments preview, and execution status. Pure ANSI escape codes — no external crates.

All entry points (REPL, CLI agent) prepend a datetime preamble to the system prompt via `build_system_prompt()` so the agent always knows the current date/time/timezone — critical for time-aware tasks.

Headless CLI agent includes `SpawnTool` (launches UDS-mode subagents) and `AgentCmdTool` (sends commands to spawned agents). Subagent timeout: 24 hours.

### UDS event bus (`quecto agent --mode uds`)

The UDS agent is the sole integration point for external consumers (TUIs, IDE plugins, web UIs). Architecture:

| Module | Responsibility |
|---|---|
| `protocol.rs` | `AgentCommand` enum (15 variants: `prompt`, `steer`, `follow_up`, `abort`, `get_state`, `get_messages`, `get_messages_tail`, `get_session_stats`, `set_model`, `get_extensions`, `reload_extensions`, `register_tools`, `unregister_tools`, `tool_result`, `clear_history`), `AgentEvent` enum (events: `agent_start`, `agent_end`, `token`, `turn_start`, `turn_end`, `tool_execution_start`, `tool_execution_end`, `response`, `execute_tool`, `extensions_changed`), `StreamingBehavior`, `SessionState`, `SessionStats`. All commands carry optional `id` for request/response correlation |
| `uds.rs` | Entry point (`run_uds_loop`), socket binding (`chmod 0600`), stale socket reaping, single-client backward-compatible path, shared dispatch loop (`dispatch_command`), system prompt injection/removal |
| `uds_multi.rs` | Multi-client accept loop (Docker-style event bus). `tokio::sync::broadcast` delivers events to all connected clients. `tokio::sync::mpsc` merges commands from all clients into a single dispatch loop (no concurrent session mutation). Max 64 clients. Agent shuts down when all clients disconnect. RAII `ClientGuard` tracks client count. Lagged clients receive a re-sync notification |
| `uds_session.rs` | `AgentSession` — in-memory state tracker (model, streaming flag, pending message queue with `VecDeque`, max 64 pending). `compute_session_stats()`, `message_to_json()`, `messages_tail_json()` |
| `uds_cancel.rs` | `CancelSlot`/`CancelHandle` state machine (Idle → Armed → Fired) for race-free steer/abort. `run_agent_prompt()` with real-time progress event forwarding. `emit_event()` helper |

Socket path: `--socket <path>` (max 104 bytes, macOS `sockaddr_un` limit) or auto-generated in `$XDG_RUNTIME_DIR` / `$TMPDIR` with UUID. Stale sockets older than 24h are reaped on startup. Socket printed to stderr: `quecto-agent-socket: <path>`.

## Dependency rule
- `domain/` imports nothing from project
- `application/` imports `domain/` only
- `infrastructure/` imports `domain/` only (implements traits)
- `interface/` imports all three (composition root)

## Commands

### `quecto` — Interactive REPL

When run with no arguments, quecto enters an interactive read-eval-print loop:

```bash
quecto
```

The REPL reads input line by line, sends each to the LLM agent, prints the response, and repeats. While the agent is processing, a live progress spinner shows current activity (thinking, tool execution with arguments and status). The spinner renders on stderr at ~12fps using pure ANSI escape codes (no external crates). Non-TTY output (pipes, CI) is silently suppressed.

| Flag | Description |
|---|---|
| `-s` / `--session` | Session name for persistence. Default: `repl:repl_default`. Use `-` for ephemeral |
| `--system` | System prompt prepended to each turn (not persisted) |
| `--model` | Override the default model from config |
| `--no-sandbox` | Disable workspace path restriction (DANGEROUS) |
| `--network` | Enable outbound network in bash (disables nsjail net namespace) |

REPL commands:

| Command | Description |
|---|---|
| `/help` | Show available commands |
| `/clear` | Clear conversation history |
| `/agent` | Manage subagent profiles (subcommands: `list`, `create`, `show`, `edit`, `remove`, `run`) |
| `/spawn` | Spawn a task as a child agent (flags: `--agent`, `--system`, `--model`, `--max-time`, `--help`) |
| `/exit` / `/quit` | Exit the REPL |

Ctrl+D (EOF) also exits cleanly.

Piped input is supported for scripting: `echo "hello" | quecto`.

### `quecto agent` — Talk to the agent

```bash
quecto agent -m "Write a Python script that generates primes"
```

| Flag | Required | Description |
|---|---|---|
| `-m` / `--message` | Yes (one-shot) | The message to send |
| `-s` / `--session` | No | Session name for persistence. Omit for `cli:default`. Use `-` for ephemeral |
| `--no-session` | No | Ephemeral mode — nothing saved or loaded (mutually exclusive with `-s`) |
| `--no-sandbox` | No | Disable workspace path restriction (DANGEROUS) |
| `--network` | No | Enable outbound network in bash (disables nsjail net namespace) |
| `--system` | No | System prompt prepended to conversation |
| `--model` | No | Override model. Accepts bare id (`gpt-5.3-codex`) or provider-qualified (`openai/gpt-4o`). Default: `gpt-5.2` |
| `--max-iterations` | No | Max tool call rounds before stopping |
| `--max-time` | No | Wall-clock timeout in seconds (exit code 2 on timeout) |
| `--mode` | No | Operation mode: default one-shot, or `uds` for UDS event bus |
| `--socket` | No | Explicit socket path for `--mode uds` (default: auto-generated in tmpdir) |
| `--persist` | No | UDS mode only — keep agent alive when all clients disconnect (default: exit on last disconnect) |
| `--effort` | No | Effort level for 4.6 models (`low`/`medium`/`high`/`max`). Overrides config and env var |
| `--disable-tool` | No | Remove a tool from the registry (repeatable). See [Disabling Tools](docs/disable-tools.md) |
| `--config` | No | Override config file path |

**Sessions** persist conversation history so the agent remembers context across runs:

```bash
quecto agent -s myproject -m "I'm working on a web scraper in Python"
quecto agent -s myproject -m "Add error handling to what we discussed"
```

Use `-s -` or `--no-session` for one-off questions that don't need history.

### `quecto agent --mode uds` — UDS event bus

For automation, long-lived agent processes, and external integrations (TUIs, IDE plugins, web UIs), use UDS mode:

```bash
quecto agent --mode uds
# stderr: quecto-agent-socket: /tmp/quecto-agent-<uuid>.sock

# Keep alive even when all clients disconnect
quecto agent --mode uds --persist
```

Multiple clients connect to the same Unix domain socket simultaneously. Events are broadcast to all connected clients; commands from all clients merge into a single dispatch loop.

Connect with any Unix socket client (e.g. `socat`) and send one JSON command per line:

```bash
socat - UNIX-CONNECT:/tmp/quecto-agent-<uuid>.sock
{"type":"prompt","id":"msg-1","message":"Summarize the CHANGELOG.md file"}
```

**Commands:**

| Type | Fields | Description |
|---|---|---|
| `prompt` | `message`, optional `id`, `streamingBehavior` | Send a user message. When agent is running, `streamingBehavior` (`"steer"` or `"followUp"`) is required |
| `steer` | `message`, optional `id` | Interrupt after current tool, deliver this message next |
| `follow_up` | `message`, optional `id` | Queue message for after current run completes |
| `abort` | optional `id` | Cancel the current agent run |
| `get_state` | optional `id` | Return session state (model, streaming, message count) |
| `get_messages` | optional `id` | Return full conversation history |
| `get_messages_tail` | `count`, optional `id` | Return last N messages |
| `get_session_stats` | optional `id` | Return token usage and cost statistics |
| `set_model` | `model` or `provider`+`modelId`, optional `id` | Switch model at runtime |
| `get_extensions` | optional `id` | Return list of registered extensions |
| `reload_extensions` | optional `id` | **Deprecated no-op** (returns success immediately) |
| `register_tools` | `tools` array, optional `id` | Register extension tools from a connected client |
| `unregister_tools` | `tools` array (names), optional `id` | Remove previously registered extension tools |
| `tool_result` | `toolCallId`, `content`, optional `isError` | Return result of an `execute_tool` request |
| `clear_history` | optional `id` | Clear conversation history, preserve system prompt |

**Events** (emitted as JSON lines):

| Type | Description |
|---|---|
| `agent_start` | Agent begins processing a prompt |
| `agent_end` | Agent finished; includes messages from this run |
| `token` | Incremental text token from streaming LLM |
| `turn_start` | New LLM call begins |
| `turn_end` | LLM call completed; includes assistant message |
| `tool_execution_start` | Tool began executing (with `toolCallId`, `toolName`, `args`) |
| `tool_execution_end` | Tool finished (with `toolCallId`, `toolName`, `result`, `isError`) |
| `execute_tool` | Routed to extension client that registered the tool (not broadcast) |
| `extensions_changed` | Broadcast when extension list changes |
| `response` | Response to a command (with `id`, `command`, `success`, optional `data`/`error`) |

### `quecto auth` — Manage API keys

```bash
# Pass token directly
quecto auth login --provider openai --token sk-proj-your-key

# Interactive: prompts you to paste the token
quecto auth login --provider anthropic

# OAuth browser flow
quecto auth login --provider openai --oauth

# Device code flow (for headless environments)
quecto auth login --provider openai --device-code

quecto auth status
quecto auth logout --provider openai
```

| Subcommand | Flags | Description |
|---|---|---|
| `auth login` | `--provider <name>` (required) | Authenticate with a provider |
| | `--token <key>` | Pass token directly (skips interactive prompt) |
| | `--oauth` | Initiate OAuth browser-based login flow |
| | `--device-code` | Initiate device code flow for headless environments |
| `auth logout` | `--provider <name>` | Remove a stored credential |
| `auth status` | | List all stored credentials with status |

Credentials are stored in `~/.quecto/credentials.json`. The credential store takes priority over keys in `config.json`.

### `quecto skills` — Manage skills

Skills are SKILL.md files with YAML frontmatter that extend the agent's system prompt with domain knowledge or instructions.

```bash
quecto skills list       # Shows name and description for each skill
quecto skills remove my-skill
quecto skills install user/repo/my-skill
```

`skills install` downloads `SKILL.md` from GitHub raw content using `<owner>/<repo>/<skill-name>` and writes it to your configured workspace:

```
<workspace>/skills/<skill-name>/SKILL.md
```

Install fails when:
- the skill path is missing or invalid
- the skill already exists locally
- the remote `SKILL.md` cannot be downloaded from `main` or `master`
- `SKILL.md` frontmatter is invalid or the skill `name` does not match `<skill-name>`

You can still add a skill manually by creating a directory under your workspace with a `SKILL.md` file:

```
<workspace>/skills/my-skill/SKILL.md
```

The `SKILL.md` file must contain YAML frontmatter with `name` and `description` fields. The body content (everything after the closing `---`) is prepended to the system prompt on every agent run. Multiple skills are concatenated.

```markdown
---
name: my-skill
description: Short description of what this skill does
license: MIT                    # optional
compatibility: opencode         # optional
metadata:                       # optional
  audience: developers
---
You are an expert at ...

## Instructions
- Do this
- Do that
```

**Frontmatter rules:**
- `name` and `description` are required (description max 1024 chars)
- `name` must match the directory name
- Names must be lowercase alphanumeric with hyphens only, 1–64 chars (e.g. `code-review`, `git-release`)
- Skills with missing or invalid frontmatter are silently skipped

### `quecto status` — Check configuration

Shows the current config, workspace path, model, and API key status. Secret values are redacted in status/debug output.

```bash
quecto status
```

### `quecto onboard` — First-time setup

Creates the default config file and workspace directory structure:

```
~/.quecto/
  config.json
  workspace/
    AGENTS.md
    IDENTITY.md
    SOUL.md
    TOOLS.md
    USER.md
```

### `quecto help` — Show usage

Prints a summary of all available commands.

```bash
quecto help
```

Also available as `quecto --help` or `quecto -h`.

### `quecto version` — Show version

Prints the version number.

```bash
quecto version
```

Also available as `quecto --version` or `quecto -v`.

## Configuration

Config file: `~/.quecto/config.json`

```json
{
  "agents": {
    "defaults": {
      "model": "gpt-5.2",
      "workspace": "~/Documents/quecto-workspace",
      "max_tokens": 8192,
      "max_tool_iterations": 999999,
      "max_session_messages": 200,
      "max_context_tokens": 190000,
      "restrict_to_workspace": true,
      "effort": "low"
    }
  },
  "providers": {
    "openai": {
      "api_key": "sk-proj-...",
      "api_base": "https://api.openai.com/v1"
    },
    "anthropic": {
      "api_key": "sk-ant-...",
      "api_base": "https://api.anthropic.com"
    }
  },
  "tools": {
    "exec": {
      "isolation": "nsjail",
      "nsjail_binary": "nsjail",
      "allow_native_fallback": false,
      "network_passthrough": false,
      "memory_limit_mb": 4096,
      "pid_limit": 256,
      "cpu_time_limit_secs": 28800,
      "wall_time_limit_secs": 14400,
      "tmp_size_mb": 512
    },
    "web": {
      "brave": {
        "enabled": true,
        "api_key": "your-brave-key",
        "max_results": 5
      },
      "duckduckgo": {
        "enabled": true,
        "max_results": 5
      }
    }
  },
  "workflow": {
    "guard_commit": true,
    "enforce_commit_after_step": 6,
    "steps": [
      { "id": 1, "label": "Update Scenarios / Add new features", "phase": "RED" },
      { "id": 2, "label": "Write/update unit tests", "phase": "RED" }
    ]
  }
}
```

All fields are optional. An empty `{}` is valid — everything uses sensible defaults.

### Provider API base overrides

Set `providers.<name>.api_base` only when you need a non-default endpoint (for example, a local mock server).

- URLs must be valid and must not include username/password, query params, or fragments.
- `https://` is required for non-local hosts (override with `QUECTO_ALLOW_CUSTOM_PROVIDER_HOSTS=1`).
- `http://` is allowed only for loopback hosts: `localhost`, `127.0.0.1`, or `::1`.
- Invalid `api_base` values cause that provider to be rejected during startup.

### Exec isolation settings

- `tools.exec.isolation`: `nsjail` (default) or `native`
- `tools.exec.nsjail_binary`: binary name or absolute path used when `isolation` is `nsjail` (default `nsjail`)
- `tools.exec.allow_native_fallback`: when `true`, missing/unexecutable nsjail falls back to native mode; when `false` (default), `bash` calls fail with a config error
- `tools.exec.network_passthrough`: allow outbound network inside nsjail (`false` by default)
- `tools.exec.memory_limit_mb`: virtual address-space limit via `--rlimit_as` (MB, default `4096`). Limits virtual reservations, not physical RSS — runtimes with large virtual mappings (Go, JVM) may need higher values
- `tools.exec.pid_limit`: max processes via `--rlimit_nproc` (default `256`). Per-UID limit, not per-jail — budget is shared across concurrent jails running as the same UID
- `tools.exec.cpu_time_limit_secs`: CPU time limit via `--rlimit_cpu` (default `28800` — 8 hours across 2 cores)
- `tools.exec.wall_time_limit_secs`: wall-clock timeout via `--time_limit` (default `14400` — 4 hours)
- `tools.exec.tmp_size_mb`: size of writable `/tmp` tmpfs inside the jail in MB (default `512`). Each concurrent jail gets its own tmpfs, so N jails consume N × `tmp_size_mb` of RAM
- `tools.exec.nsjail_binary`: must resolve to an executable under trusted system paths (`/usr/bin`, `/bin`, `/usr/sbin`, `/sbin`, `/usr/local/bin`); relative paths are rejected
- exec child environment is allowlisted by default (`PATH`, locale vars, and basic shell/runtime vars), preventing broad secret env leakage

nsjail resource limits use rlimits (`--rlimit_as`, `--rlimit_nproc`, `--rlimit_cpu`) instead of cgroups, so no root access or cgroup write permissions are required. The cgroup namespace is always disabled (`--disable_clone_newcgroup`). This means nsjail works in containers, on unprivileged users, and in any environment without `/sys/fs/cgroup/` access.

nsjail mounts `/bin`, `/usr`, `/lib`, `/lib64` read-only inside the jail, plus individual `/etc` files needed by the dynamic linker and NSS (`ld.so.cache`, `ld.so.conf`, `nsswitch.conf`, `passwd`, `group`, `ssl`, `alternatives`). Only paths that exist on the host are mounted.

### Environment variable overrides

| Variable | Overrides |
|---|---|
| `QUECTO_BASE_DIR` | Base directory (default `~/.quecto`) |
| `QUECTO_AGENTS_DEFAULTS_MODEL` | `agents.defaults.model` |
| `QUECTO_AGENTS_DEFAULTS_MAX_TOKENS` | `agents.defaults.max_tokens` |
| `QUECTO_AGENTS_DEFAULTS_TEMPERATURE` | `agents.defaults.temperature` |
| `QUECTO_AGENTS_DEFAULTS_WORKSPACE` | `agents.defaults.workspace` |
| `QUECTO_AGENTS_DEFAULTS_MAX_SESSION_MESSAGES` | `agents.defaults.max_session_messages` |
| `QUECTO_MAX_CONTEXT_TOKENS` | `agents.defaults.max_context_tokens` |
| `QUECTO_AGENTS_DEFAULTS_EFFORT` | `agents.defaults.effort` (`low`/`medium`/`high`/`max`; invalid values ignored) |
| `QUECTO_PROVIDERS_OPENAI_API_KEY` | `providers.openai.api_key` |
| `QUECTO_PROVIDERS_ANTHROPIC_API_KEY` | `providers.anthropic.api_key` |
| `QUECTO_OFFLINE` | Set to `1`/`true`/`yes` to disable auto-download of tool binaries (rg, fd) |
| `QUECTO_ALLOW_CUSTOM_PROVIDER_HOSTS` | Set to `1` to allow non-default HTTPS hosts for provider API bases |

## Tools

The agent has access to tools it can call autonomously to accomplish tasks.

Tool definitions are cached in the registry at registration time (sorted once, reused for subsequent definition lookups).

External tool binaries (`rg`, `fd`) are resolved via `ensure_tool`: system PATH → cache dir (`~/.local/share/quecto/tools/`) → auto-download from GitHub releases. Set `QUECTO_OFFLINE=1` to disable downloads.

| Tool | Description |
|---|---|
| `bash` | Execute a shell command. Per-invocation timeout, 1 MiB stdout/stderr capture, dangerous commands blocked. Supports `commandPrefix` for environment setup. Output truncated with Pi-compatible notices |
| `read` | Read file contents (text or image). Text: 2000-line / 50KB truncation with offset/limit pagination. Images (jpg/png/gif/webp): base64-encoded, auto-resized to 2000px max dimension. Magic-byte MIME detection |
| `write` | Create or overwrite a file (auto-creates parent directories) |
| `edit` | Replace text in a file. Two-stage exact→fuzzy matching, CRLF/BOM preservation, no-op detection, LCS-based unified diff output |
| `ls` | List directory contents. Case-insensitive sort, `/` suffix for directories, configurable limit (default 500, max 5000), 50KB output cap |
| `grep` | Search file contents with ripgrep (`rg --json`). Regex or literal, case-insensitive option, context lines from file cache, 100-match / 50KB limit, 500-char line truncation |
| `find` | Find files by glob pattern with fd. Respects nested `.gitignore` files, path-segment patterns via `--full-path`, configurable limit (default 1000), 50KB output cap |
| `recall` | Retrieve a previously collapsed tool output by its spill ID (e.g. `turn20:bash:0`). Use `recall("list")` for the full index |
| `spawn` | Spawn a background UDS-mode subagent for long-running tasks |
| `agent_cmd` | Send commands (`steer`, `follow_up`, `abort`, `get_state`) to spawned UDS subagents |
| `web_search` | Search the web via Brave Search or DuckDuckGo |
| `web_fetch` | Fetch a URL and return its content as readable text (HTML stripped by default) |
| `workflow` | Manage the BDD/TDD development workflow (status, check, uncheck, reset, skip, set_issue, clear_issue). `WorkflowGuard` blocks `git commit`/`git push` when steps are incomplete |

Filesystem tools (`read`, `write`, `edit`, `ls`) run on async `tokio::fs` adapters.

## Security

The agent operates inside a sandbox:

- **Workspace restriction**: When `restrict_to_workspace` is `true` (default), all file operations are confined to the workspace directory. Symlinks pointing outside are blocked. Path traversal (`../`) is caught.
- **Dangerous commands blocked**: `rm -rf /`, `rm -r -f /`, `mkfs`, `dd`, `shutdown`, `reboot`, `chmod -R 777 /`, fork bombs, and pipe-to-shell patterns (`curl|sh`) are always blocked regardless of other settings. Command checks normalize whitespace/casing, so equivalent variants like `rm  -rf /` are also blocked.
- **Exec runtime isolation**: The `bash` tool runs in `nsjail` mode by default with rlimit-based resource bounds (no cgroup access required); `native` remains available as an explicit opt-in via `tools.exec.isolation`.
- **Environment isolation**: `QUECTO_*` environment variables (including API keys) are stripped from child processes spawned by the `bash` tool.
- **Secret redaction**: Log/status output redacts OpenAI/Anthropic (`sk-*`), Groq (`gsk_*`/`gsk-*`), and Telegram bot token values.
- **UDS socket security**: Socket files are created with `chmod 0600` (owner-only). Stale sockets older than 24h are reaped on startup.

## Provider Fallback

Quecto supports OpenAI, Anthropic, and ChatGPT Codex as LLM providers. OpenAI and Anthropic support SSE streaming (`chat_stream()` and `chat_stream_incremental()`) for incremental response assembly, with automatic fallback to non-streaming mode. The Codex provider uses the Responses API with SSE streaming.

If multiple providers are configured, automatic fallback applies:

- Tries the primary provider first
- On rate-limit or server errors, falls back to the secondary provider
- Authentication errors (wrong API key) do not trigger fallback
- Providers enter a cooldown period after failures
- Classification is provider-scoped (`DomainError::Provider`), using extracted HTTP status codes first, then semantic message matching
- Model routing: use `provider/model` syntax (e.g. `anthropic/claude-sonnet-4-20250514`) to target a specific provider

### ChatGPT Codex provider

OAuth tokens from `auth.openai.com` (obtained via `quecto auth login --provider openai --oauth`) are routed to the ChatGPT Codex backend using the Responses API. Features:

- SSE streaming with accumulator-based response assembly
- `prompt_cache_key` support: session keys are FNV-1a hashed with type-prefix preservation for privacy (e.g. `telegram:12345` → `telegram:c3d7e1f2`)
- Orphaned tool call pair repair: mismatched `function_call`/`function_call_output` pairs (from context pruning or mid-turn interruption) are detected and dropped before sending to the API
- Parallel tool calls enabled

### OAuth auto-refresh

OAuth-backed providers are wrapped in `RefreshableProvider` so that expired tokens are automatically refreshed mid-session on 401. The decorator intercepts auth errors, refreshes the token via the credential store, rebuilds the inner provider with the new token, and retries the request once.

API key resolution order: credential store (`quecto auth login`) > config file > environment variable.

## Development workflow

1 - Update Scenarios / Add new features
2 - Write/update unit tests (run a quick smoke check; full suite runs on push)
3 - Ensure new/modified tests FAIL (RED) — quick targeted run only, not full suite
4 - Implement code (GREEN)
5 - Commit
6 - Push (pre-push hook will run tests and linting)
7 - Create PR
8 - Despatch sub agents in parallel as reviewers (Architecture, Security and Performance)
9 - Fix all valid review concerns
10 - Push changes to remote
11 - Reply to the reviewers comments on the PR and mark resolved (use graphql)
12 - Run pre-merge hooks (real-LLM, machete, deny)
13 - Merge
14 - Move to local master and pull

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

## Testing

```bash
# Core suite (no real provider calls)
cargo test --test bdd

# Core suite (24-way sharded, fastest local full run)
bash scripts/run-bdd-shards.sh --suite non-real-bdd --shards 24 --timeout 12m

# Real-LLM smoke subset (CI-sized)
bash scripts/run-bdd-shards.sh --suite real-llm-smoke --shards 24 --timeout 12m --tag real-llm-smoke --real-llm

# Real-LLM full suite
bash scripts/run-bdd-shards.sh --suite real-llm-bdd --shards 24 --timeout 12m --tag real-llm --real-llm
```

`scripts/pre-push.sh` runs quality checks plus a parallel test wave (`cargo test --lib`, `cargo test --test architecture`, and 24-way sharded non-real BDD), caches successful runs per `HEAD` commit + script hash, and writes a full log to `.git/pre-push.last.log`.

Pre-push controls:
- `QUECTO_E2E_TIMEOUT` timeout per BDD shard (default `12m`)
- `QUECTO_BDD_SHARDS` shard count for non-real BDD (default `24`)
- `QUECTO_PREPUSH_FORCE=1` to bypass cache and rerun all checks

Pre-merge controls (real-LLM lane):
- `QUECTO_REAL_LLM_TIMEOUT` timeout per real-LLM shard (default `12m`)
- `QUECTO_REAL_LLM_SHARDS` shard count for real-LLM BDD (default `24`)
- `QUECTO_REAL_LLM_TAG` scenario tag to run (default `real-llm`; use `real-llm-smoke` for smoke)
- `QUECTO_PREMERGE_FORCE=1` to bypass cache and rerun merge-time checks

Coverage is intentionally not part of git hooks. Run coverage in nightly CI (recommended with `cargo llvm-cov`) to keep local dev loops fast.

## Directory Structure

```
~/.quecto/
  config.json              # Main configuration
  credentials.json         # Stored API tokens (from quecto auth)
  sessions/                # Persisted conversation history (safe filename mapping)
    cli_default.json
    repl_repl_default.json
  workspace/
    skills/                # Skill definitions (YAML frontmatter required)
      my-skill/
        SKILL.md
    ...                    # Agent working directory (files created by the agent)
```

Tool binary cache (auto-downloaded `rg`, `fd`):
```
~/.local/share/quecto/tools/
  rg
  fd
```

## Documentation

| Guide | Description |
|---|---|
| [Getting Started](docs/getting-started.md) | Quickstart guide for UDS agent integration |
| [UDS Protocol](docs/uds-protocol.md) | Complete UDS command and event specification |
| [Sessions](docs/sessions.md) | Conversation persistence, context management, spill/recall |
| [Extensions](docs/extensions.md) | Add custom tools via native extensions (config-gated) or UDS extensions (external processes) |
| [Subagents](docs/subagents.md) | Spawning and controlling UDS-mode subagents with `spawn` and `agent_cmd` tools |
| [Disabling Tools](docs/disable-tools.md) | Restricting which tools the agent can access via `--disable-tool` |
| [Workflow](docs/workflow.md) | Configurable BDD/TDD step-by-step development workflow with guards |

## Tech stack
Rust 2024, Tokio, reqwest+rustls, serde/serde_json, uuid, tracing, dirs, thiserror, similar, base64, sha2, flate2, tar, rand, urlencoding, unicode-normalization. Dev: cucumber 0.21, futures, tempfile, wiremock 0.6, regex.

## License

MIT
