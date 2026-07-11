# Quecto

Quecto is a Rust workspace centred on a lightweight personal AI assistant. The main `quecto` binary receives messages via the command line or a UDS event bus, routes them through an LLM (OpenAI, Anthropic, or ChatGPT Codex), executes tools (shell commands, file operations, search), and persists conversations to disk.

The workspace also includes companion binaries for terminal UI access (`quecto-tui`), HTTP/WebSocket gateway access (`quecto-api`), MCP tool bridging (`quecto-mcp`), and managed runtime orchestration (`quecto-runtime-manager`). Quecto runs on a VPS, small Linux host, or container with no non-Rust application runtime.

## Release Notes

Current version: **0.91.4**.

## Quick Start

```bash
# Install both binaries (from the repository root). quecto-tui starts the
# kernel by running `quecto`, so the `quecto` binary must be on PATH unless
# you connect by --socket. The `quecto` binary is built by the
# quecto-agentic-harness package.
cargo install --path quecto-agentic-harness
cargo install --path quecto-tui

# Store your API key (zero-config: no setup step — defaults apply,
# and a config file is optional)
quecto auth login --provider openai --token sk-proj-your-key

# Talk to the agent (one-shot)
quecto agent -m "Hello, what can you do?"

# Or start an interactive session
quecto

# Or launch the terminal UI client. This starts `quecto agent --mode uds`
# as the local kernel process and connects to it automatically.
quecto-tui

# Workflow-driven launch: prompt the model to enter workflow mode immediately
quecto-tui --workflow --workflow-guards
```

`quecto-tui` is a lightweight terminal UI for the UDS agent. By default it
spawns `quecto agent --mode uds` for you, then connects over the same framed JSON
protocol documented below. In that normal mode, the workflow tool is available
but dormant: the model is not instructed to start a workflow until you ask it to
select a template. You can also point the TUI at an already-running agent with
`--socket /path/to/agent.sock`.

For local development without installing, either put Cargo's build output on
`PATH` before running the TUI, or start the kernel yourself and connect to its
socket:

```bash
# Option A: let the TUI spawn the kernel from target/debug/quecto
cargo build -p quecto-agentic-harness -p quecto-tui
PATH="$PWD/target/debug:$PATH" cargo run -p quecto-tui --

# Option B: run the kernel explicitly, then attach the TUI from another terminal
cargo run -p quecto-agentic-harness -- agent --mode uds --socket /tmp/quecto.sock --persist
cargo run -p quecto-tui -- --socket /tmp/quecto.sock
```

In these examples, "kernel" means the root `quecto` process running
`quecto agent --mode uds`. It owns the model session, tools, credentials,
workflow state, and Unix socket. `quecto-tui` is only a client for that socket.

The **first launch** right after `cargo install` can be slower because the
freshly written `quecto` binary is cold in the OS page cache, so the kernel
takes longer to start. `quecto-tui` therefore waits up to **30s** for the agent
socket on a direct launch before failing (and, on timeout, suggests running
`quecto --version` once to warm the binary, then retrying). `scripts/run-tui.sh`
pre-warms `quecto --version` before launching the TUI so the cold-binary cost is
paid up front.

Useful `quecto-tui` flags:

| Flag | Description |
|---|---|
| `--socket <path>` | Connect to an existing UDS agent instead of spawning one |
| `--workflow` | Start the spawned agent in workflow-driven mode immediately |
| `--workflow-guards` | Enable workflow bash guards for the spawned agent; does not by itself force workflow prompt injection |
| `--no-workflow` | Disable workflow tool/state/prompt for the spawned agent |
| `--system <prompt>` | Pass a custom system prompt to the spawned agent |
| `--config <path>` | Use an alternate quecto config file when spawning the agent |
| `--no-sandbox` | Spawn the agent with filesystem sandboxing disabled |

Handy TUI controls: `Shift+Enter` or `Alt+Enter` inserts a newline, `Escape`
aborts the active run (or clears the editor when idle), `Ctrl+C` clears the
editor first and otherwise aborts the active run, `Ctrl+L` opens the model
selector, and `Ctrl+O` toggles tool output expansion. Slash commands include
`/model`, `/clear`, `/new`, `/session`, `/workflow-auto`, `/workflow-nudge`,
`/help` (also `/hotkeys`), and `/quit` (also `/exit`). See
[`quecto-tui/README.md`](../quecto-tui/README.md) for a dedicated TUI reference.

## Workspace binaries

| Binary | Package | Purpose |
|---|---|---|
| `quecto` | root package | Main CLI, REPL, one-shot agent, and persistent UDS event bus |
| `quecto-tui` | `quecto-tui` | Lightweight terminal UI client that spawns or connects to a UDS agent |
| `quecto-api` | `quecto-api` | HTTP/WebSocket gateway for a running UDS agent; see [`quecto-api/README.md`](../quecto-api/README.md) |
| `quecto-mcp` | `quecto-mcp` | UDS extension that discovers MCP tools, registers them with Quecto, and proxies tool calls; see [`quecto-mcp/README.md`](../quecto-mcp/README.md) |
| `quecto-runtime-manager` | `quecto-runtime-manager` | HTTP runtime manager for provisioning and supervising isolated Quecto runtimes |

## Architecture

Four layers, strict dependency direction. Inner layers never import outer.

```
interface/ --> application/ --> domain/
                    |
infrastructure/ ----+
```

### domain/ — Pure types and traits
Zero deps except `thiserror` and `serde` (derive). Defines system vocabulary.

| File | Purpose |
|---|---|
| `message.rs` | `Message` (constructors `::system/user/assistant/tool`; pruning fields: `turn`, `is_pinned`, `is_manifest`, `is_collapsed`, `tool_name`, `input_preview`, `spill_id`; `image_blocks` for tool results, `user_image_blocks` for user images, `is_error`, `stop_reason`, `thinking_blocks` for extended thinking replay), `Role`, `ToolCall`, `LlmResponse` (with `thinking_blocks`), `UsageInfo`, `StopReason` (maps `model_context_window_exceeded` → `MaxTokens`, `pause_turn` → `EndTurn`, `sensitive` → `Error`), `UserImageBlock`, `ThinkingBlock` (`Normal` with thinking text + signature, `Redacted` with opaque data) |
| `provider.rs` | `LlmProvider` trait (dyn-compatible), `ChatRequest` (with `session_id` for prompt caching, `cancel_flag`, `thinking_level`, `tool_choice`, `metadata`, `effort`), `chat()` + `chat_stream()` (SSE with non-streaming fallback) + `chat_stream_incremental()` (real-time `StreamEvent` channel), `CancelFlag`, `ThinkingLevel`, `ToolChoice`, `RequestMetadata`, `EffortLevel` (with `parse()` / `as_str()`) |
| `tool.rs` | `Tool` trait, `ToolRegistry` trait, `ToolGuard` trait, `ToolDefinition` (with `Cow<'static, str>` fields), `ToolResult` (with `image_blocks` for base64 images), `ImageBlock` |
| `agent.rs` | `AgentLoop` trait, `AgentInfo`, `AgentResult`, `AgentProgressEvent` (with `tool_call_id` on `ToolStarted`/`ToolFinished`, `Token` for streaming, `Thinking` with context stats), `ProgressCallback` |
| `session.rs` | `Session`, `SessionStore` trait, `SpillEntry`, `SpillIndex`, `ContextSpillStore` trait, `strip_tool_history()`, `filter_orphan_tool_pairs()` (with `OrphanDiag`) |
| `extension.rs` | `Extension` trait (`name()`, `tools()`, `system_prompt_snippet()`) |
| `subagent.rs` | `SubagentConfig`, `validate_agent_id()` |
| `workflow.rs` | `WorkflowEngine`, `WorkflowConfig` (`auto_continue`, `completion_nudge`, `templates`), `WorkflowTemplate`, `WorkflowTemplateStep`, `WorkflowGuardRule`, `WorkflowRun`, `WorkflowRunPersisted`, `WorkflowMode` (SelectingTemplate/Active/Complete), `WorkflowSnapshot`, `WorkflowError`, `default_templates()` (single `feature` Quecto workflow). UDS-only; available by default in UDS as a dormant tool, prompt-driven via `--workflow`, disabled via `--no-workflow` |
| `error.rs` | `DomainError` enum (Provider, Tool, Session, Security, Config, Other) |

Traits use `Pin<Box<dyn Future + Send + '_>>` for `Arc<dyn Trait>` compatibility.

### application/ — Use cases
Depends only on `domain/`. Orchestration logic, no I/O.

| File | Purpose |
|---|---|
| `agent_loop.rs` | Core LLM-tool loop: send → execute tools → repeat. Traces `tool_name`, `duration_ms`, `is_error`. Progress callbacks for REPL spinner. Supports incremental streaming via `chat_stream_incremental()`. Passes configured `effort` level through to every `ChatRequest` |
| `context_pruning.rs` | Token estimation, pinned spill manifest, tool-call-count tool-result collapse, conversation-message collapse (`context_collapse_after_messages`, default 50), and the demotion-ladder ceiling (stub, then drop; `pin_recent_turns = 2` tail is never demoted). Current config defaults: `max_context_tokens = 200000`, `context_collapse_after_tool_calls = 50`, `context_collapse_after_messages = 50`; set a collapse knob to `4294967295` (`u32::MAX`) to disable it |
| `reload.rs` | `/reload` use case: strips stale tool history via `strip_tool_history()`, clears spill index, coordinates `SessionStore` + `ContextSpillStore` |
| `subagent.rs` | `SubagentContext` — child agent contexts with inherited sandbox |

### infrastructure/ — Concrete adapters
Implements domain traits with real I/O (serde, reqwest, tokio, filesystem).

| Component | Contents |
|---|---|
| `config.rs` | `Config` with serde, env overrides (`QUECTO_AGENTS_DEFAULTS_EFFORT` validated at load), `WorkflowConfig` (template library, optional custom templates). Tolerates unknown fields (forward-compatible) |
| `providers/` | `OpenAiProvider` (SSE streaming via `openai_sse`), `AnthropicProvider` (SSE streaming via `anthropic_sse`, extended thinking support with `signature_delta` capture, auto-enables adaptive thinking for 4.6 models, effort default `low` for 4.6 models, OAuth identity for tokens — system prompt prefix + tool name remapping + beta headers, `interleaved-thinking` + `fine-grained-tool-streaming` betas, thinking block replay in multi-turn via `ThinkingBlock`, `claude_code.rs` for tool name canonical casing), `CodexProvider` (Responses API, SSE, `prompt_cache_key`, orphan pair repair), `RefreshableProvider` (OAuth 401 → auto-refresh → retry), `FallbackProvider` (cooldown + error classification + `provider/model` routing syntax). URL validation: https required for non-loopback (override with `QUECTO_ALLOW_CUSTOM_PROVIDER_HOSTS=1`) |
| `tools/` | `bash/` (shell, 1MiB cap, per-invocation timeout, `commandPrefix`, native exec), `filesystem/` (`ReadTool` with image base64+auto-resize, `WriteTool`, `EditTool` with fuzzy match+CRLF/BOM+LCS diff, `LsTool` with limit+case-insensitive sort), `grep.rs` (rg JSON output, file-cache context), `find.rs` (fd, nested .gitignore, path-segment globs via `--full-path`), `spawn.rs` (background UDS-mode subagent spawning), `agent_cmd.rs` (send commands to spawned UDS agents — `steer`, `follow_up`, `abort`, `get_state`), `web_search.rs` (Brave+DDG), `web_fetch.rs` (URL fetch with HTML stripping, per-host SSRF allowlist for tests), `recall.rs` (spill retrieval), `docs.rs` (`DocsTool` — quecto's capability docs embedded via `include_str!`, served by the `docs` tool from any directory), `workflow_tool.rs` (`WorkflowTool` thin façade over `WorkflowEngine`, available by default in UDS unless `--no-workflow`; `WorkflowGuard` template-aware `ToolGuard` impl — mutating actions emit `workflow_state` events, guard registration gated by `--workflow-guards`), `path_utils.rs`, `truncate.rs`, `command_match.rs`, `registry.rs` (`ToolRegistryImpl`, `guard_count()`) |
| `persistence/` | `FileSessionStore` (round-trips all Message fields including `thinking_blocks` for multi-turn thinking replay), `FileContextSpillStore` (JSONL append-only) |
| `security/` | `Sandbox` — workspace path validation + command filtering |
| `extensions/` | `ExtensionRegistry` (register extensions, aggregate tools + system prompt snippets), `NativeExtension` (compiled-in config-gated tools, e.g. `web_search`, `web_fetch`), `UdsExtensionTool` (routes tool execution to connected UDS clients via mpsc/oneshot channels). See [Extensions guide](docs/extensions.md) |
| `auth/` | `CredentialStore` (file-based, `AuthMethod::Token`/`OAuth`), `oauth.rs` (browser + device code flows, Anthropic OAuth, OpenAI account ID extraction from JWT) |
| `logging.rs` | `redact_api_keys()` — pattern-based secret redaction |

### Tool isolation

**Filesystem tools** (`read`, `write`, `edit`, `ls`): `Sandbox::validate_path` — canonicalises the path, follows symlinks at every component, and rejects anything outside `canonical_workspace`. Called before any I/O.

**bash** (exec only): commands run natively as the invoking user, with the workspace as the working directory but **no filesystem confinement** — unlike the filesystem tools, `bash` is *not* restricted to the workspace and can read any path the user can (e.g. `~/.ssh`, `~/.aws`, `/etc/passwd`) and reach the network. `Sandbox::validate_command` rejects a denylist of obviously-destructive commands, but this is a **best-effort speed-bump, not a security boundary** (trivially bypassed via shell escapes, `base64`, env indirection). There are **no in-process resource limits** (memory/PID/CPU/wall-time are unbounded). Real isolation is delegated to the deployment — see [Security](#security).

**Tool binary resolution** (`rg`, `fd`): `grep` and `find` use binaries already available on `PATH` and report direct installation guidance when missing.

### interface/ — CLI (composition root)
Manual arg parsing (no clap). Entry point: `cli::run(args) -> i32`.

| Command | Description |
|---|---|
| `quecto` | Interactive REPL (`-s` session, `--system` prompt, `--model` override, `--no-sandbox`; global `--config <path>`) with live progress spinner |
| `quecto agent -m <msg>` | Headless one-shot (`-s`, `--no-session`, `--system`, `--model`, `--max-iterations`, `--max-time`, `--effort`, `--disable-tool`, `--no-sandbox`; global `--config <path>`) |
| `quecto agent --mode uds` | Persistent UDS event bus: multi-client length-prefixed JSON protocol over Unix domain socket (`--socket <path>` for explicit path, auto-generated otherwise; `--persist`, `--workflow`, `--workflow-guards`, `--no-workflow` supported) |
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
| `protocol.rs` | `AgentCommand` enum (18 variants: `prompt`, `steer`, `follow_up`, `abort`, `get_state`, `get_messages` (optional `count`), `get_session_stats`, `list_sessions`, `resume_session`, `set_model`, `get_extensions`, `reload_extensions`, `register_tools`, `unregister_tools`, `tool_result`, `clear_history`, `set_workflow_automation`, `get_subagents`), `AgentEvent` enum (events: `agent_start`, `agent_end`, `token`, `turn_start`, `turn_end`, `tool_execution_start`, `tool_execution_end`, `response`, `execute_tool`, `extensions_changed`, `subagent_notification`, `subagent_state_changed`, `workflow_state`), `StreamingBehavior`, `SessionState`, `SessionStats`. All commands except `tool_result` carry optional `id` for request/response correlation |
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
| `--config <path>` | Override config file path (global option) |

REPL commands:

| Command | Description |
|---|---|
| `/help` | Show available commands |
| `/clear` | Clear conversation history |
| `/agent` | Manage subagent profiles (subcommands: `list`, `create`, `show`, `edit`, `remove`, `run`) |
| `/spawn` | Spawn a task as a child agent (flags: `--agent`, `--system`, `--model`, `--max-time`, `--help`) |
| `/exit` / `/quit` | Exit the REPL |

Ctrl+D (EOF) also exits cleanly.

Quectoped input is supported for scripting: `echo "hello" | quecto`.

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
| `--system` | No | System prompt prepended to conversation |
| `--model` | No | Override model. Accepts bare id (`gpt-5.3-codex`) or provider-qualified (`openai/gpt-4o`). Default: `gpt-5.5` |
| `--max-iterations` | No | Max tool call rounds before stopping |
| `--max-time` | No | Wall-clock timeout in seconds (exit code 2 on timeout) |
| `--mode` | No | Operation mode: default one-shot, or `uds` for UDS event bus |
| `--socket` | No | Explicit socket path for `--mode uds` (default: auto-generated in tmpdir) |
| `--persist` | No | UDS mode only — keep agent alive when all clients disconnect (default: exit on last disconnect) |
| `--workflow` | No | UDS mode only — start workflow-driven prompt injection immediately |
| `--workflow-guards` | No | UDS mode only — enable workflow bash command guards; does not force prompt injection |
| `--no-workflow` | No | UDS mode only — explicitly disable workflow tool/state/prompt |
| `--parent-id` | No | UDS mode only — declares this agent's parent in the unit tree; stamped as `parent_id` on its `workflow_state` events. Set automatically by `spawn`; rarely passed by hand |
| `--effort` | No | Reasoning effort level (`none`/`low`/`medium`/`high`/`xhigh`/`max`). OpenAI reasoning models take the documented OpenAI scale (`none`–`xhigh`); Anthropic 4.6 models take `low`/`medium`/`high`/`max`. Unknown values are rejected. Overrides config and env var |
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
{"type":"prompt","id":"msg-1","message":"Summarize the README.md file"}
```

**Commands:**

| Type | Fields | Description |
|---|---|---|
| `prompt` | `message`, optional `id`, `streamingBehavior` | Send a user message. When agent is running, `streamingBehavior` (`"steer"` or `"followUp"`) is required |
| `steer` | `message`, optional `id` | Interrupt after current tool, deliver this message next |
| `follow_up` | `message`, optional `id` | Queue message for after current run completes; if idle, run it immediately |
| `abort` | optional `id` | Cancel the current agent run |
| `get_state` | optional `id` | Return session state (model, streaming, message count, and workflow snapshot when enabled) |
| `get_messages` | optional `count`, optional `id` | Return conversation history (omit `count` for all, `N` for the last N messages) |
| `get_session_stats` | optional `id` | Return token usage and cost statistics |
| `list_sessions` | optional `id` | Return persisted CLI sessions available for resume |
| `resume_session` | `session`, optional `id` | Switch the active UDS conversation to a persisted CLI session |
| `set_model` | `model` or `provider`+`modelId`, optional `id` | Switch model at runtime |
| `get_extensions` | optional `id` | Return list of registered extensions |
| `reload_extensions` | optional `id` | **Deprecated no-op** (returns success immediately) |
| `register_tools` | `tools` array, optional `id` | Register extension tools from a connected client |
| `unregister_tools` | `tools` array (names), optional `id` | Remove previously registered extension tools |
| `tool_result` | `toolCallId`, `content`, optional `isError` | Return result of an `execute_tool` request |
| `clear_history` | optional `id` | Clear conversation history, preserve system prompt |
| `set_workflow_automation` | optional `id`, `autoContinue`, `completionNudge` | Toggle core workflow auto-continue/completion nudges for this UDS session |
| `get_subagents` | optional `id` | Return spawned subagents and live status. Each entry includes `readOnly` to identify observer sub-agents spawned with write/edit disabled |

**Events** (emitted as length-prefixed JSON frames):

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
| `subagent_notification` | Passive child-agent completion/error/exit notification for UI visibility |
| `subagent_state_changed` | Broadcast replacement snapshot of spawned subagent statuses, including `readOnly` observer status |
| `workflow_state` | Broadcast when workflow mode/progress/template state changes |
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

### `quecto status` — Check configuration

Shows the current config, workspace path, model, and API key status. Secret values are redacted in status/debug output.

```bash
quecto status
```

### First run — zero config

quecto needs no setup step. With no config file it runs on defaults; supply a
key via `quecto auth login` or `QUECTO_*` env vars. A config file is optional —
when present it's read from `~/.quecto/config.json`, and the workspace is
created on demand:

```
~/.quecto/
  config.json     # optional — defaults apply when absent
  workspace/       # agent working directory
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
      "model": "gpt-5.5",
      "workspace": "~/Documents/quecto-workspace",
      "max_tokens": 8192,
      "max_tool_iterations": 999999,
      "max_session_messages": 200,
      "context_collapse_after_tool_calls": 50,
      "max_context_tokens": 300000,
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
    },
    "openai_compatible": {
      "endpoints": [
        {
          "prefix": "spark",
          "api_key": "sk-local-or-bearer-token",
          "api_base": "http://127.0.0.1:8000/v1",
          "allow_remote_http": false
        }
      ]
    }
  },
  "tools": {
    "web": {
      "brave": {
        "enabled": true,
        "api_key": "your-brave-key",
        "max_results": 5
      },
      "duckduckgo": {
        "enabled": true,
        "max_results": 5
      },
      "fetch": {
        "enabled": false,
        "max_response_kb": 32
      }
    }
  },
  "workflow": {
    "auto_continue": true,
    "completion_nudge": true,
    "templates": []
  }
}
```

All fields are optional. An empty `{}` is valid — everything uses sensible defaults. `effort` is optional and is unset by default; Anthropic 4.6 requests default to `low` effort when unset, and OpenAI reasoning requests omit the field so the server default applies. For a workflow template example, see [`examples/config.json`](examples/config.json).

### Provider API base overrides

Set `providers.<name>.api_base` only when you need a non-default endpoint (for example, a local mock server).

- URLs must be valid and must not include username/password, query params, or fragments.
- `https://` is required for non-local built-in provider hosts (override with `QUECTO_ALLOW_CUSTOM_PROVIDER_HOSTS=1`).
- `http://` is allowed only for loopback hosts: `localhost`, `127.0.0.1`, or `::1`.
- Invalid `api_base` values cause that provider to be rejected during startup.

### OpenAI-compatible custom endpoints

Use `providers.openai_compatible.endpoints` when you need one or more OpenAI-compatible endpoints alongside a normal OpenAI/ChatGPT OAuth credential:

```json
{
  "providers": {
    "openai": { "api_key": "" },
    "openai_compatible": {
      "endpoints": [
        {
          "prefix": "spark",
          "api_key": "sk-local-or-bearer-token",
          "api_base": "http://127.0.0.1:8000/v1",
          "allow_remote_http": false
        }
      ]
    }
  },
  "agents": {
    "defaults": { "model": "spark/qwen3" }
  }
}
```

Each endpoint registers an OpenAI-compatible provider named by `prefix`, so `spark/qwen3` routes to that endpoint and sends `Authorization: Bearer <api_key>`. These endpoints do not read the credential store and never switch to Codex routing.

For tailnet/LAN HTTP endpoints, set `allow_remote_http: true` on that endpoint or set `QUECTO_ALLOW_CUSTOM_PROVIDER_HOSTS=1`. HTTPS custom hosts are allowed for `openai_compatible` endpoints.

### Runtime model/provider registry (`models.json`)

Use `~/.quecto/models.json` for community-extensible providers and model metadata. The running agent watches both `config.json` and `models.json`; opening `/model`, changing model, or sending the next turn reloads changes without restarting quecto or quecto-tui.

Auth modes are explicit provider keys. The built-in vendor slots are split so quecto never silently switches between OAuth monthly-plan credentials and token-billed API keys:

- `openai-api/...` uses `providers.openai.api_key` / `OPENAI_API_KEY`.
- `openai-oauth/...` uses the stored `quecto auth login openai` credential.
- `anthropic-api/...` uses `providers.anthropic.api_key` / `ANTHROPIC_API_KEY`.
- `anthropic-oauth/...` uses the stored `quecto auth login anthropic` credential.

Community providers use the same explicit model. API-key providers are fully data-driven. OAuth providers may reference only kernel-known OAuth identities (`openai`, `anthropic`); adding a brand-new OAuth identity requires kernel code because OAuth client IDs, scopes, token URLs, and refresh handling are security-sensitive.

Example: Anthropic API key alongside Anthropic OAuth:

```json
{
  "providers": {
    "anthropic-api": {
      "api": "anthropic-messages",
      "baseUrl": "https://api.anthropic.com",
      "auth": { "mode": "apiKey", "apiKey": "$ANTHROPIC_API_KEY" },
      "models": [
        { "id": "claude-opus-4-8", "name": "Claude Opus 4.8 (API)" }
      ]
    },
    "anthropic-oauth": {
      "api": "anthropic-messages",
      "auth": { "mode": "oauth", "oauthProvider": "anthropic" },
      "models": [
        { "id": "claude-opus-4-8", "name": "Claude Opus 4.8 (OAuth)" }
      ]
    }
  }
}
```

Example: OpenAI-compatible API provider with slashful model IDs:

```json
{
  "providers": {
    "fireworks": {
      "api": "openai-completions",
      "baseUrl": "https://api.fireworks.ai/inference/v1",
      "auth": { "mode": "apiKey", "apiKey": "$FIREWORKS_API_KEY" },
      "models": [
        { "id": "accounts/fireworks/models/glm-5p2", "name": "GLM 5.2" }
      ]
    }
  }
}
```

Supported provider fields: `api` (`openai-completions` or `anthropic-messages`), `baseUrl`/`apiBase`, `auth`, `authHeader`, `allowRemoteHttp`, and `models`. API keys support `$ENV` and `${ENV}` interpolation. Supported model fields include `id`, `name`, `reasoning`, `input`, `contextWindow`, `maxTokens`, and `cost` (`input`, `output`, `cacheRead`, `cacheWrite`).

To use an OAuth-backed registry provider, first run `quecto auth login openai` or `quecto auth login anthropic`, then select the registry provider key (for example `/model anthropic-oauth/claude-opus-4-8`). The `/model` selector shows `[apiKey]` or `[oauth]` so the billing/auth mode is visible before selection.

`providers.openai_compatible.endpoints` remains supported for OpenAI-compatible API-key endpoints, but `models.json` is preferred when you want those models to appear in `/model`.

### Exec behaviour

- `bash` commands run natively in the workspace via the user's shell. The shell is read from `$SHELL` and validated against an allowlist of known system shells (defaults to `/bin/sh`).
- exec child processes clear the ambient environment and then explicitly receive the current process environment (or test-provided overrides).
- `Sandbox::validate_command` rejects a denylist of destructive commands (e.g. `rm -rf /`, recursive `chown root`) before execution; this stays active unless `--no-sandbox` is passed.
- There is no built-in process/network/resource isolation. For untrusted workloads, run Quecto inside a container (or other OS-level sandbox), which bounds filesystem, network, and resource access for the whole process.

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
| `QUECTO_AGENTS_DEFAULTS_EFFORT` | `agents.defaults.effort` (`none`/`low`/`medium`/`high`/`xhigh`/`max`; unknown values are rejected at config load with an error naming the valid values) |
| `QUECTO_TOOLS_WEB_BRAVE_API_KEY` | `tools.web.brave.api_key` |
| `OPENAI_API_KEY` | `providers.openai.api_key` |
| `ANTHROPIC_API_KEY` | `providers.anthropic.api_key` |
| `QUECTO_ALLOW_CUSTOM_PROVIDER_HOSTS` | Set to `1` to allow custom provider hosts, including remote HTTP for explicit OpenAI-compatible endpoints |

## Tools

The agent has access to core tools plus optional config-gated tools and UDS extension tools it can call autonomously to accomplish tasks.

Tool definitions are cached in the registry at registration time (sorted once, reused for subsequent definition lookups).

External tool binaries (`rg`, `fd`) are resolved from `PATH`; missing binaries return direct installation guidance.

| Tool | Description |
|---|---|
| `bash` | Execute a shell command. Per-invocation timeout, 1 MiB stdout/stderr capture, dangerous commands blocked. Supports `commandPrefix` for environment setup. Output truncated with compatible notices |
| `read` | Read file contents (text or image). Text: 2000-line / 50KB truncation with offset/limit pagination. Images (jpg/png/gif/webp): base64-encoded, auto-resized to 2000px max dimension. Magic-byte MIME detection |
| `write` | Create or overwrite a file (auto-creates parent directories) |
| `edit` | Replace text in a file. Two-stage exact→fuzzy matching, CRLF/BOM preservation, no-op detection, LCS-based unified diff output |
| `ls` | List directory contents. Case-insensitive sort, `/` suffix for directories, configurable limit (default 500, max 5000), 50KB output cap |
| `grep` | Search file contents with ripgrep (`rg --json`). Regex or literal, case-insensitive option, context lines from file cache, 100-match / 50KB limit, 500-char line truncation |
| `find` | Find files by glob pattern with fd. Respects nested `.gitignore` files, path-segment patterns via `--full-path`, configurable limit (default 1000), 50KB output cap |
| `recall` | Retrieve a spilled tool output by its spill ID (e.g. `turn20:bash:0`). Use `recall("list")` for the full index |
| `spawn` | Spawn a background UDS-mode subagent for long-running tasks |
| `agent_cmd` | Send commands to spawned UDS subagents: `prompt`, `steer`, `follow_up`, `abort`, `kill`, `await`, `get_state`, `get_messages` (optional `count` — omit for all, N for last N), `get_session_stats`, `get_subagents`, `get_extensions`, `set_model`, `clear_history`, `reload_extensions` |
| `web_search` | Optional: search the web via Brave Search or DuckDuckGo when `tools.web.brave.enabled` or `tools.web.duckduckgo.enabled` is true |
| `web_fetch` | Optional: fetch a URL and return readable text when `tools.web.fetch.enabled` is true (HTML stripped by default; `raw: true` returns the original body) |
| `workflow` | UDS-only template-based development workflow (status, list_templates, select_template, check, uncheck, skip, reset, set_issue, clear_issue, check_guards with command). Available by default in UDS as a dormant tool unless `--no-workflow`; `--workflow` starts prompt-driven mode immediately. See [Workflow docs](docs/workflow.md) |

Filesystem tools (`read`, `write`, `edit`, `ls`) run on async `tokio::fs` adapters.

## Security

Quecto provides **in-process guardrails for the filesystem tools only**; real isolation is the deployment's job (run it in a container). Understand the split before exposing it to untrusted input:

- **Workspace restriction (filesystem tools)**: When `restrict_to_workspace` is `true` (default), `read`/`write`/`edit`/`ls` are confined to the workspace. Symlinks pointing outside are blocked; path traversal (`../`) is caught.
- **`bash` is NOT confined**: the exec tool runs commands natively as the invoking user. Its working directory is the workspace, but it can read any path the user can (`~/.ssh`, `~/.aws`, cloud/`git` credentials, `/etc/passwd`) and reach the network. It has **no resource limits** (memory/PID/CPU/wall-time are unbounded). The `Sandbox::validate_command` denylist (`rm -rf /`, `mkfs`, `dd`, fork bombs, `curl|sh`, …) is a **best-effort speed-bump, not a security boundary** — trivially bypassed via shell escapes/`base64`/env indirection. Do not rely on it to contain untrusted commands.
- **Isolation is the container's responsibility**: to run untrusted workloads safely, run Quecto in a container that is **non-root**, with **minimal/read-only mounts** (so `bash` can't read host secrets), **cgroup resource limits** (`--memory`, `--pids-limit`, `--cpus`), and a **network policy** (drop egress unless needed). A default `docker run` enforces none of these. (Old config files' `tools.exec` isolation/nsjail keys are now ignored.)
- **Environment handling**: `bash` children are launched with `env_clear()` and then receive the current process environment (or explicit test overrides). Do not place secrets in the Quecto process environment if the agent should not be able to read them with shell commands.
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
- Model routing: use `provider/model` syntax (e.g. `anthropic/claude-sonnet-4-6`) to target a specific provider

### ChatGPT Codex provider

OAuth tokens from `auth.openai.com` (obtained via `quecto auth login --provider openai --oauth`) are routed to the ChatGPT Codex backend using the Responses API. Features:

- SSE streaming with accumulator-based response assembly
- `prompt_cache_key` support: session keys are FNV-1a hashed with type-prefix preservation for privacy (e.g. `telegram:12345` → `telegram:c3d7e1f2`)
- Orphaned tool call pair repair: mismatched `function_call`/`function_call_output` pairs (from context pruning or mid-turn interruption) are detected and dropped before sending to the API
- Parallel tool calls enabled

### OAuth auto-refresh

OAuth-backed providers are wrapped in `RefreshableProvider` so that expired tokens are automatically refreshed mid-session on 401. The decorator intercepts auth errors, refreshes the token via the credential store, rebuilds the inner provider with the new token, and retries the request once.

API key resolution order: credential store (`quecto auth login`) > environment variable > config file.

## Development workflow

Quecto development uses the repository-local Quecto workflow checklist:

Pure-move refactors (for example file extractions, renames, or byte-identical moves) should ship in their own PR so reviewers can distinguish structural movement from behavior changes. That standalone refactor PR may land before or after the behavioral change that motivates it.

1 - Install/check local quality hooks
2 - Update Scenarios / Add new features
3 - Write/update unit tests (run a quick smoke check; full suite runs on push)
4 - Ensure new/modified tests FAIL (RED) — quick targeted run only, not full suite
5 - Despatch three BDD review finders (Gherkin discipline, Falsifiability, Coverage)
6 - Implement code (GREEN)
7 - Refactor (perf, security, clean arch)
8 - Ensure tests still pass (GREEN)
9 - Bump semver for every changed crate and sync version docs
10 - Commit
11 - Push (pre-push hook will run tests and linting)
12 - Create PR
13 - Despatch narrow parallel review finders, verify adversarially, post one review
14 - Fix all valid review concerns
15 - Push changes to remote
16 - Reply to the reviewers comments on the PR and mark resolved (use graphql)
17 - Verify the PR meets every issue acceptance criterion
18 - Confirm the pre-push gate passed and report the PR (do NOT merge)
19 - Clean up sub agents

## Quality gates

| Gate | Command |
|---|---|
| Quality scripts | `scripts/check-quality.sh`, `scripts/check-bdd-quality.sh` |
| Format | `cargo fmt --check` |
| Lint | `cargo clippy -p quecto -- -D warnings` (zero warnings) |
| Unit tests | `cargo test -p quecto --no-fail-fast --lib 2>&1 \| scripts/test-filter.sh` |
| Architecture | `cargo test -p quecto --no-fail-fast --test architecture 2>&1 \| scripts/test-filter.sh` |
| BDD (sharded) | See [Sharded BDD](#sharded-bdd-24-way-parallel) below |

All test commands pipe through `scripts/test-filter.sh` which strips the per-test `... ok` noise and shows only:
- **Summary totals** (passed/failed counts)
- **Failure details** (test name, file:line, assertion message, panic reason)
- **BDD failures** with Feature/Scenario context

`--no-fail-fast` ensures all failures are reported in a single run, not just the first.

Two-tier hooks: pre-commit (~20-40s: quality+fmt+clippy) and pre-push (tests + 24-shard BDD + coverage + machete + deny + the zero-cost mocked e2e suite `@mock-llm`). The paid `@manual-real-llm` suite is NOT run on push by default; opt in on demand with `QUECTO_RUN_REAL_LLM=1 git push`. SHA-based caching. Install via `scripts/install-hooks.sh`.

### Sharded BDD (24-way parallel)

Non-real-LLM (fast, no API key needed):
```bash
(for i in $(seq 0 23); do
  (timeout 12m env QUECTO_BDD_SHARD_INDEX=$i QUECTO_BDD_SHARD_TOTAL=24 cargo test -p quecto --no-fail-fast --features test-support --test bdd 2>&1 | scripts/test-filter.sh) &
done
wait)
```

Provider smoke (paid, opt-in, minimal live request):
```bash
QUECTO_PROVIDER_SMOKE=1 QUECTO_TAG=provider-smoke cargo test -p quecto --no-fail-fast --features test-support --test bdd 2>&1 | scripts/test-filter.sh
```

Provider smoke runs only provider-specific scenarios with available credentials: OpenAI uses `OPENAI_API_KEY`, Anthropic uses `ANTHROPIC_API_KEY`, and Codex uses an existing OpenAI OAuth credential in the `quecto` credential store. Missing provider credentials filter out that provider's smoke scenario without failing unrelated smoke checks.

Legacy live behavioral suites are tagged `@manual-real-llm` and still gated by `QUECTO_REAL_LLM=1`, but behavioral e2e coverage should prefer mocked provider responses so normal test runs do not incur provider costs.

### Running individual features or scenarios

To debug a single scenario, add a temporary tag (e.g. `@focus`) to the scenario in the `.feature` file, then run:
```bash
QUECTO_TAG=focus cargo test -p quecto --no-fail-fast --features test-support --test bdd 2>&1 | scripts/test-filter.sh
```
Remove the tag before committing.

## Testing

```bash
# Core suite (no real provider calls)
cargo test -p quecto --features test-support --test bdd

# Core suite (24-way sharded, fastest local full run)
bash scripts/run-bdd-shards.sh --suite non-real-bdd --shards 24 --timeout 12m

# Mocked e2e suite (free, deterministic, default pre-push e2e lane — no API key)
bash scripts/run-bdd-shards.sh --suite mock-llm-bdd --shards 24 --timeout 12m --tag mock-llm

# Provider smoke subset (paid, opt-in; filters providers without credentials)
QUECTO_PROVIDER_SMOKE=1 QUECTO_TAG=provider-smoke cargo test -p quecto --no-fail-fast --features test-support --test bdd

# Live Real-LLM full suite (paid, manual/on-demand — needs OPENAI_API_KEY in .env)
bash scripts/run-bdd-shards.sh --suite real-llm-bdd --shards 24 --timeout 12m --tag manual-real-llm --real-llm
```

The e2e suite exists in two parallel lanes that assert the same behaviours:
- **`@mock-llm` (default, free):** WireMock-backed deterministic coverage under `tests/features/e2e_mock_llm*.feature` plus the full `@manual-real-llm` behavioral mirror. Makes zero paid provider calls and passes with no API key. This is what every `git push` runs.
- **`@manual-real-llm` (manual, paid):** the retired live behavioral suite under `tests/features/e2e_real_llm*.feature`, retained for occasional exploratory end-to-end validation. Run it on demand with the command above, or fold it into a push via `QUECTO_RUN_REAL_LLM=1 git push`.

Contributor rules for the live/mock e2e split:
- Keep behavioral scenarios dual-tagged with `@manual-real-llm @mock-llm` when they should run in both lanes.
- Mock only external provider HTTP responses in the `@mock-llm` lane. Do not synthesize app-level events such as UDS `agent_end`, `token`, `workflow_state`, or `get_state` responses.
- Preserve scenario inputs used by the live/manual lane. If mock routing needs a test-only provider alias, keep it behind `test-support`, `QUECTO_TAG=mock-llm`, and loopback mock URLs rather than editing live scenario text.
- Put provider protocol edge cases in provider/unit tests. BDD e2e mocks should stay focused on application behavior: tools run, files change, sessions persist, REPL/UDS/subprocess output appears, and workflow events are produced by real app paths.
- When a scenario asserts tool or workflow behavior, the mocked provider must return the relevant tool-call response(s) before the final text marker. Do not make those scenarios pass by returning only the marker text.
- For UDS workflow scenarios, use the real multi-client socket path when asserting broadcast-only events. The test harness should read the socket while the run is active to avoid backpressure on large workflow event streams.
- Keep `@provider-smoke` tiny and live-provider only: it validates credentials/provider availability, not tools, sessions, workflow, REPL, or UDS behavior.

`scripts/pre-push.sh` runs quality checks plus a parallel test wave (`cargo test -p quecto-agentic-harness --lib`, plus the `architecture`, `contracts`, and `repo_docs` integration test targets, and 24-way sharded non-real BDD), then the zero-cost mocked e2e lane, caches successful runs per `HEAD` commit + script hash, and writes a full log to `.git/pre-push.last.log`. A `.env` provider key alone never triggers paid calls - the paid `@manual-real-llm` lane runs only under the explicit `QUECTO_RUN_REAL_LLM=1` opt-in.

Pre-push controls:
- `QUECTO_E2E_TIMEOUT` timeout per BDD shard (default `12m`)
- `QUECTO_BDD_SHARDS` shard count for non-real BDD (default `24`)
- `QUECTO_MOCK_LLM_SHARDS` / `QUECTO_MOCK_LLM_TIMEOUT` shard count / timeout for the mocked e2e lane (defaults `24` / `12m`)
- `QUECTO_RUN_REAL_LLM=1` opt in to also run the live paid `@manual-real-llm` suite on push
- `QUECTO_PREPUSH_FORCE=1` to bypass cache and rerun all checks

Pre-merge controls (real-LLM lane):
- `QUECTO_PROVIDER_SMOKE=1` enables `@provider-smoke` live provider checks (excluded by default)
- `OPENAI_API_KEY` supplies the OpenAI API smoke credential
- `ANTHROPIC_API_KEY` supplies the Anthropic API smoke credential
- An existing OpenAI OAuth credential in the `quecto` credential store enables the Codex smoke scenario
- `QUECTO_REAL_LLM_TIMEOUT` timeout per real-LLM shard (default `12m`)
- `QUECTO_REAL_LLM_SHARDS` shard count for real-LLM BDD (default `24`)
- `QUECTO_REAL_LLM_TAG` scenario tag to run (default `manual-real-llm`; use `real-llm-smoke` for the old smoke subset)
- `QUECTO_PREMERGE_FORCE=1` to bypass cache and rerun merge-time checks

Coverage runs in the full pre-push/pre-merge gate. For manual checks without pushing, use `cargo llvm-cov` (or `scripts/pre-push.sh` for the canonical full local gate); expect that path to take longer than the commit-time hook.

## Directory Structure

```
~/.quecto/
  config.json              # Main configuration
  credentials.json         # Stored API tokens (from quecto auth)
  sessions/                # Persisted conversation history (safe filename mapping)
    cli_default.json
    repl_repl_default.json
  workspace/                # Agent working directory (files created by the agent)
```

## Documentation

| Guide | Description |
|---|---|
| [Quecto Agent Capability Guide](docs/quecto.md) | Compact retrieval map for agents using Quecto capabilities on demand |
| [Getting Started](docs/getting-started.md) | Quickstart guide for UDS agent integration |
| [UDS Protocol](docs/uds-protocol.md) | Complete UDS command and event specification |
| [Sessions](docs/sessions.md) | Conversation persistence, context management, spill/recall |
| [Extensions](docs/extensions.md) | Add custom tools via native extensions (config-gated) or UDS extensions (external processes) |
| [Subagents](docs/subagents.md) | Spawning and controlling UDS-mode subagents with `spawn` and `agent_cmd` tools |
| [Disabling Tools](docs/disable-tools.md) | Restricting which tools the agent can access via `--disable-tool` |
| [Workflow](docs/workflow.md) | UDS-only template-based workflow engine with default dormant tool availability, selector mode, guards, and live prompt injection |

## Tech stack
Rust 2024, Tokio, reqwest+rustls, serde/serde_json, uuid, tracing, dirs, thiserror, similar, base64, sha2, rand, urlencoding, macOS unicode-normalization. Dev: cucumber 0.21, futures, tempfile, wiremock 0.6, regex.

## License

Proprietary (`LicenseRef-Proprietary`). This is a private repository; all rights reserved unless explicitly stated otherwise.
