# Quecto

Quecto is a Rust workspace centred on a lightweight personal AI assistant. The main `quecto` binary receives messages via the command line or a UDS event bus, routes them through an LLM (OpenAI, Anthropic, or ChatGPT Codex), executes tools (shell commands, file operations, search), and persists conversations to disk.

The workspace also includes companion binaries for terminal UI access (`quecto-tui`), HTTP/WebSocket gateway access (`quecto-api`), MCP tool bridging (`quecto-mcp`), and managed runtime orchestration (`quecto-runtime-manager`). Quecto runs on a VPS, small Linux host, or container with no non-Rust application runtime.

## Quick Start

```bash
# Install both binaries from source (from the repository root). quecto-tui starts
# the kernel by running `quecto`, so the `quecto` binary must be on PATH unless
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

Handy TUI controls: `Shift+Enter` or `Alt+Enter` inserts a newline, `Escape`
aborts the active run (or clears the editor when idle), `Ctrl+C` clears the
editor first and otherwise aborts the active run, `Ctrl+L` opens the model
selector, and `Ctrl+O` toggles tool output expansion. Safe `http(s)` markdown
links are OSC 8 hyperlinks; with mouse capture on for scroll/selection, use
**`Shift+click`** (Alacritty and many other terminals; some use Ctrl/Cmd+click)
to open a link in the browser — plain click selects text. Slash commands include
`/model`, `/clear`, `/new`, `/session`, `/workflow-auto`, `/workflow-nudge`,
`/help` (also `/hotkeys`), and `/quit` (also `/exit`). See
[`quecto-tui/README.md`](../quecto-tui/README.md) for a dedicated TUI reference
(including mouse/link gestures; `/help` is the in-app source of truth).

## Workspace binaries

| Binary | Package | Purpose |
|---|---|---|
| `quecto` | `quecto-agentic-harness` | Main CLI, REPL, one-shot agent, and persistent UDS event bus |
| `quecto-tui` | `quecto-tui` | Lightweight terminal UI client that spawns or connects to a UDS agent |
| `quecto-api` | `quecto-api` | HTTP/WebSocket gateway for a running UDS agent; see [`quecto-api/README.md`](../quecto-api/README.md) |
| `quecto-mcp` | `quecto-mcp` | UDS extension that discovers MCP tools, registers them with Quecto, and proxies tool calls; see [`quecto-mcp/README.md`](../quecto-mcp/README.md) |
| `quecto-runtime-manager` | `quecto-runtime-manager` | HTTP runtime manager for provisioning and supervising isolated Quecto runtimes |
| *(library)* | `quecto-line-io` | Shared bounded length-prefixed / legacy-line UDS framing (8 MiB cap) |
| *(library)* | `quecto-image` | What an image is, owned in one place and shared by the agent (UDS commands, `read`, the token estimate) and `quecto-api`: MIME allowlist, sniffing, size limit, base64, header parsing, admission |

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
| `agent_loop.rs` | Core LLM-tool loop: send → execute tools → repeat. Traces `tool_name`, `duration_ms`, `is_error`. Progress callbacks for interactive agent clients. Supports incremental streaming via `chat_stream_incremental()`. Passes configured `effort` level through to every `ChatRequest` |
| `context_pruning.rs` | Token estimation, the pinned spill manifest and the emergency ladder (`context_pruning_ceiling.rs`: stub, then drop; the system prompt, the manifest, the in-flight prompt and the `pin_recent_turns = 2` tail are never demoted; it prunes down to a 75% low-water mark and drops stubs only when stubbing cannot meet the ceiling). **The watermark pass is the only context mode** (#2414; `agent_loop/context_watermark.rs`): the context only grows at the end, and once the request reaches `context_high_tokens` (256000) one cut takes it down to `context_low_tokens` (70000), keeping the system messages, the brief, the latest prompt and the newest whole exchanges, and archiving the rest behind one stub that `recall("archive")` lists. The ceiling still wins (H is at most the effective budget; L scales with it). The ladder runs only when no cut can bring a request under the ceiling, logged as `context_pruned` with `watermark_fallback`. Each cut writes a `context_cut` event-log record. For every agent, a cut or dropped `read` result makes the next read of that file answer its content again. Current config defaults: `max_context_tokens = 300000`, `swarm_max_context_tokens = 300000` (the budget once the process takes part in a swarm), `context_high_tokens = 256000`, `context_low_tokens = 70000`. The old pruning keys (`context_mode`, `context_collapse_*`) are refused at load; see [sessions.md](docs/sessions.md#context-management) |
| `reload.rs` | `/reload` use case: strips stale tool history via `strip_tool_history()`, clears spill index, coordinates `SessionStore` + `ContextSpillStore` |
| `sessions/` | The sessions capability (#1968): `use_cases/` (list, history, recovery, sync, report, save, clear, rewind, fresh, resume, recall/retain), `ports.rs` + `ports/` (the `SessionStore`, `ContextSpillStore`, export and runtime ports, each contract-tested), `dto/`, and the one `ActiveSessionState`. Constructed only by `composition/`; the interface parses, maps and presents. See [docs/sessions.md](docs/sessions.md#architecture-epic-1968) |
| `subagent.rs` | `SubagentContext` — child agent contexts with inherited tool policy |

### infrastructure/ — Concrete adapters
Implements domain traits with real I/O (serde, reqwest, tokio, filesystem).

| Component | Contents |
|---|---|
| `config.rs` | `Config` with serde, env overrides (`QUECTO_AGENTS_DEFAULTS_EFFORT` validated at load), `WorkflowConfig` (template library, optional custom templates). Tolerates unknown fields (forward-compatible) |
| `providers/` | `OpenAiProvider` (SSE streaming via `openai_sse`; user images as `image_url` parts at `detail: high`, a tool batch's images in one user message after it, `openai_images.rs`; animated GIFs are sent as a marker on both OpenAI wires), `AnthropicProvider` (SSE streaming via `anthropic_sse`, extended thinking support with `signature_delta` capture, auto-enables adaptive thinking for 4.6 models, effort default `low` for 4.6 models, OAuth identity for tokens — system prompt prefix + tool name remapping + beta headers, `interleaved-thinking` + `fine-grained-tool-streaming` betas, thinking block replay in multi-turn via `ThinkingBlock`, `claude_code.rs` for tool name canonical casing), `CodexProvider` (Responses API, SSE, images as `input_image` data URLs at `detail: high` in user content and `function_call_output` arrays, `prompt_cache_key` and, on ChatGPT OAuth, the matching `session_id` header, encrypted reasoning kept and replayed to the endpoint, account and model that produced it, orphan pair repair), `RefreshableProvider` (OAuth 401 → auto-refresh → retry), `FallbackProvider` (cooldown + error classification + `provider/model` routing syntax). URL validation: https required for non-loopback (override with `QUECTO_ALLOW_CUSTOM_PROVIDER_HOSTS=1`) |
| `tools/` | `bash/` (shell, 1MiB cap, per-invocation timeout, `commandPrefix`, native exec), `filesystem/` (`ReadTool` with image base64+auto-resize, `WriteTool`, `EditTool` with fuzzy match+CRLF/BOM+LCS diff, `LsTool` with limit/offset over the sorted whole directory), `grep.rs` (rg JSON output, file-cache context; `grep_request.rs` validates the rg options, `grep_listing.rs` renders `--null` files/count listings), `find_fd.rs` (fd effect, nested .gitignore, VCS internals skipped, file/directory type, workspace-relative paths, path-segment globs via `--full-path`, bounded concurrent pipe collection and cancellation cleanup), `spawn.rs` (background UDS-mode subagent spawning), `agent_cmd.rs` (send commands to spawned UDS agents — `steer`, `follow_up`, `abort`, `get_state`), `web_search.rs` (Brave+DDG), `http/web_fetch.rs` (URL fetch adapter: client built from an owned recipe with no proxy, DNS answers pinned to the checked public addresses, every redirect hop checked, #1942), `recall.rs` (spill retrieval), `docs.rs` (`DocsTool` — quecto's capability docs embedded via `include_str!`, served by the `docs` tool from any directory), `workflow_tool.rs` (`WorkflowTool` thin façade over `WorkflowEngine`, available by default in UDS unless `--no-workflow`; `WorkflowGuard` template-aware `ToolGuard` impl — mutating actions emit `workflow_state` events, guard registration gated by `--workflow-guards`), `path_utils.rs`, `truncate.rs`, `command_match.rs`, `registry.rs` (`ToolRegistryImpl`, `guard_count()`) |
| `persistence/` | `FileSessionStore` (round-trips all Message fields including `thinking_blocks` for multi-turn thinking replay), `FileContextSpillStore` (JSONL append-only) |
| `security/` | `Sandbox` — shared filesystem path hook + command filtering |
| `extensions/` | `ExtensionRegistry` (register extensions, aggregate tools + system prompt snippets), `NativeExtension` (compiled-in config-gated tools, e.g. `web_search`, `web_fetch`), `UdsExtensionTool` (routes tool execution to connected UDS clients via mpsc/oneshot channels). See [Extensions guide](docs/extensions.md) |
| `auth/` | `CredentialStore` (file-based, `AuthMethod::Token`/`OAuth`), `oauth.rs` (browser + device code flows, Anthropic OAuth, OpenAI account ID extraction from JWT) |
| `logging.rs` | `redact_api_keys()` — pattern-based secret redaction |

### Tool isolation

**Parallel tool calls** (#2169): when every call in one model response is to a bundled tool that reads only and says its calls may overlap (`read`, `grep`, `find`, `ls`, `web_fetch`, `docs`), the calls run at once; a response with any other call (`bash`, `write`, `edit`, `spawn`, `swarm`, `agent_cmd`, `recall`, `web_search` (paced by its provider's rate limit), or any extension tool) runs its calls one at a time, as before. A plain `grep` may overlap; one with `rank_by` does not (it paces a rate-limited judge), and neither may two identical calls. Results are added to the conversation, the audit log and the event log in call order. A turn cancelled mid-batch keeps the results that finished.

**Filesystem tools** (`read`, `write`, `edit`, `ls`): call `Sandbox::validate_path` as a shared path hook before I/O. It no longer confines paths to the workspace; filesystem tools can access any path the Quecto process user can access.

**bash** (exec only): commands run natively as the invoking user, with the workspace as the working directory but **no filesystem confinement** — unlike the filesystem tools, `bash` is *not* restricted to the workspace and can read any path the user can (e.g. `~/.ssh`, `~/.aws`, `/etc/passwd`) and reach the network. `Sandbox::validate_command` rejects a denylist of obviously-destructive commands, but this is a **best-effort speed-bump, not a security boundary** (trivially bypassed via shell escapes, `base64`, env indirection). There are **no in-process resource limits** (memory/PID/CPU/wall-time are unbounded). Real isolation is delegated to the deployment — see [Security](#security). Output is captured up to `exec_max_capture_bytes` per stream (1 MiB by default), keeping the start and the true end with the dropped middle named; once the shell exits, the call returns when the output has been quiet for half a second (at most 5 s for a job that keeps writing) even if a background job (`cmd &`) still holds it, and says so; the job keeps running and its later output is discarded (#2167).

**Find ownership:** `application/agent_turn/use_cases/find.rs` owns typed invocation and limit normalization; `interface/tools/find.rs` translates tool JSON/results; `infrastructure/tools/find_fd.rs` owns path resolution and fd processes; `composition/find.rs` alone assembles the graph. Native registration consumes the assembled tool. Incomplete discovery is reported explicitly; output hints remain outside the 50KiB payload cap.

**Event log** (#2150): set `"telemetry": {"event_log": {"enabled": true}}` in config (off by default) and every agent writes a detailed log, including sub-agents and swarm members in containers (their `~/.quecto` is the host's). Each session's log is `<base_dir>/audit/<session>.jsonl` (keyed by process for a session with no key), one JSON record per line: each model request (start and finish, tokens including cache, cost, attempts, outcome), each turn's start and end (the end records `cached_input_tokens`, the share of `input_tokens` the provider's prompt cache served, whenever the provider reports it: Anthropic's cache reads, OpenAI's and Codex's `cached_tokens`, and `cache_write_tokens`, Anthropic's cache writes), each tool call's arguments (and the model's raw text when the harness had to replace invalid arguments) and each tool result's outcome, duration (policy and approval waits included) and sizes, context pruning, sub-agent commands, provider errors and, for the Rust swarm board, one `swarm_op` per board op (its outcome and refusal kind, duration, lock wait, busy wait and whether the lock was busy, filed under no turn: ids, kinds and sizes only, no board text; see [docs/swarm.md](docs/swarm.md#telemetry)). Every record carries its time (ISO and Unix milliseconds), process id, host (the machine's name, or for a container member the container's, `quecto-<environment>` with the bundled container scripts, which tells containers apart since their process ids repeat), session (`cli:<name>` for a named session, `cli:default` for an unnamed one-shot `-m` run) and, for a sub-agent, `parent`: the parent as it was named to the child, which is the parent's own `session` for an unnamed parent and `cli:<parent>` otherwise. **The log holds tool arguments and result previews as sent**, so it is private: the directory is 0700 and its files 0600. A session's log stops at 256 MiB with one final `log_capped` record. A run stopped by `--max-time` ends its log with its unfinished request (`request_observed`, outcome `cancelled`) and an `error` record (source `deadline`), both written with turn 0 (after the run). A session asked to leave nothing behind (`-s -`, `--no-session`) keeps none. Your global config's switch covers every agent, one started with its own `--config` (a container member) included. Links are never followed, and only a regular file is a log. With the switch off nothing is opened or written, except the audit log a `--workflow` session has always kept. A repository overlay may switch the event log on, never off.

**Grep ranking and the search log** (#2136): with `tools.grep.relevance.enabled` set and a TypeSafe key (`TYPESAFE_API_KEY`, else `~/.config/typesafe/api_key`), the `grep` tool offers `rank_by`, a description of what the agent is looking for. Every match rg reads (a ranked search reads up to `max_candidates` matches, with 16 MiB of rg's output as a backstop, where a plain search reads the matches it shows and one more, with 4 MiB as a backstop (#2163); a ranked search cut short says so) is judged by TypeSafe's Jev, up to `max_candidates` (default 1000, at most 5000: a guard against runaway fan-out, calls cost next to nothing), `concurrency` at a time (default 32, at most 64), and returned best first with its score. When every match was read and judged and none scores 0.3 or more, a note says none looks relevant. A rejected key stops judging at once; a rate-limited request is retried briefly. **Ranking sends each judged match's path and lines (with three lines of context either side) to api.typesafe.ai**, which is why only the global config can enable it: a repository overlay may set `tools.grep.relevance` only to `{"enabled": false}`, and may not switch the search log off. If ranking is not configured, is set outside its range, times out (`timeout_secs`, default 30, at most 120; scores that arrived in time are kept) or fails, the search still returns rg's order with a one-line note. Every search is recorded in `<base_dir>/search-log/<YYYY-MM-DD>.jsonl`, whether or not ranking is on: its arguments, output mode, matches found (the matches read: for a plain content search at most the match limit plus one, so a count over the limit means more exist than were shown; counts logged before 0.107.120 are not comparable), completeness, error, time and any ranking with its scores. The arguments are kept as sent (cut at 2000 characters), so a pattern that holds a secret is recorded too. The files are private to you (directory 0700, files 0600), one per UTC day, and can be deleted at any time. Results from the log itself are always dropped (rg may still read it), however its directory is reached. Set `tools.grep.log.enabled` to `false` to stop recording. Settings live in config as `"tools": {"grep": {"relevance": {"enabled": true, "model": "jev-latest", "max_candidates": 1000, "timeout_secs": 30, "concurrency": 32}, "log": {"enabled": true}}}`.

**Tool binary resolution** (`rg`, `fd`): `grep` and `find` use binaries already available on `PATH` and report direct installation guidance when missing.

### interface/ — CLI (composition root)
Manual arg parsing (no clap). Entry point: `cli::run(args) -> i32`.

| Command | Description |
|---|---|
| `quecto` | Interactive login, setup, and configuration shell |
| `quecto agent -m <msg>` | Headless one-shot (`-s`, `--no-session`, `--system`, `--model`, `--max-iterations`, `--max-time`, `--effort`, `--disable-tool`; global `--config <path>`) |
| `quecto agent --mode uds` | Persistent UDS event bus: multi-client length-prefixed JSON protocol over Unix domain socket (`--socket <path>` for explicit path, auto-generated otherwise; `--persist`, `--workflow`, `--workflow-guards`, `--no-workflow` supported) |
| `quecto status` | Config summary, provider availability |
| `quecto auth login\|logout\|status` | Credential management (token/OAuth/device-code) |
| `quecto help\|version` | Self-explanatory |

The REPL delegates authentication and configuration commands to the existing CLI handlers; it does not construct an agent or tool runtime.

Headless CLI agent includes `SpawnTool` (launches UDS-mode subagents) and `AgentCmdTool` (sends commands to spawned agents). Subagent timeout: 24 hours.

### UDS event bus (`quecto agent --mode uds`)

The UDS agent is the sole integration point for external consumers (TUIs, IDE plugins, web UIs). Architecture:

| Module | Responsibility |
|---|---|
| `protocol.rs` | `AgentCommand` enum (documented surface: `prompt`, `steer`, `follow_up`, `abort`, `get_state`, `get_messages` (optional `count`/`before`/`agent_id`), `get_message` (`messageId` plus optional `offset`/`limit`/`agent_id`/`toolCallId` for bounded recovery), `get_session_stats`, `list_models`, `list_sessions`, `new_session`, `resume_session`, `set_model`, `set_effort`, `get_tool_catalogue`, `reload`, `register_tools`, `unregister_tools`, `tool_result`, `clear_history`, `rewind_to`, `set_workflow_automation`, `get_subagents`), `AgentEvent` enum (events: `agent_start`, `agent_end` (`messageRefs`; legacy `messages` empty after #1060), `workflow_idle`, `token`, `turn_start`, `turn_end` (`messageRefs` / context occupancy fields), `tool_execution_start`, `tool_execution_end`, `response`, `execute_tool`, `tool_catalogue_changed`, `subagent_notification`, `subagent_state_changed`, `subagent_messages_appended`, `workflow_state`), `StreamingBehavior`, `SessionState` (includes `effort` / `effortLevels` / `maxContextTokens`), `SessionStats` (includes `contextTokens` / `maxContextTokens`). All commands except `tool_result` carry optional `id` for request/response correlation |
| `uds.rs` | Entry point (`run_uds_loop`), socket binding (`chmod 0600`), stale socket reaping, single-client backward-compatible path, shared dispatch loop (`dispatch_command`), system prompt injection/removal |
| `uds_multi.rs` | Multi-client accept loop (Docker-style event bus). `tokio::sync::broadcast` delivers events to all connected clients. `tokio::sync::mpsc` merges commands from all clients into a single dispatch loop (no concurrent session mutation). Max 64 clients. Agent shuts down when all clients disconnect. RAII `ClientGuard` tracks client count. Lagged clients receive a re-sync notification |
| `uds_session.rs` | `AgentSession` — in-memory state tracker (model, streaming flag, usage, pending message queue with `VecDeque`, max 64 pending; it holds no session key — presenters read the active session's identity). `compute_session_stats()`, `message_to_json()`, `history_page_json()` |
| `uds_cancel.rs` | `CancelSlot`/`CancelHandle` state machine (Idle → Armed → Fired) for race-free steer/abort. `run_agent_prompt()` with real-time progress event forwarding. `emit_event()` helper |

Socket path: `--socket <path>` (max 104 bytes, macOS `sockaddr_un` limit) or auto-generated in `$XDG_RUNTIME_DIR` / `$TMPDIR` with UUID. Stale sockets older than 24h are reaped on startup. Socket printed to stderr: `quecto-agent-socket: <path>`.

## Dependency rule
- `domain/` imports nothing from project
- `application/` imports `domain/` only
- `infrastructure/` imports `domain/` only (implements traits)
- `interface/` imports all three (composition root)

## Commands

### `quecto` — Setup and configuration REPL

Running `quecto` with no arguments opens a small interactive shell for login, initial setup, and configuration inspection. Supported commands are `auth login`, `auth logout`, `auth status`, `status`, `models discover <provider-key>`, `help`, and `exit`. Agent prompts, tools, workflows, subagents, chat sessions, and progress rendering are intentionally unavailable. Use `quecto agent` for one-shot/UDS agents or `quecto-tui` for an interactive agent interface.

### `quecto agent` — Talk to the agent

```bash
quecto agent -m "Write a Python script that generates primes"
```

| Flag | Required | Description |
|---|---|---|
| `-m` / `--message` | Yes (one-shot) | The message to send |
| `-s` / `--session` | No | Session name for persistence. Omit for `cli:default`. Use `-` for ephemeral |
| `--no-session` | No | Ephemeral mode — nothing saved or loaded (mutually exclusive with `-s`) |
| `--system` | No | System prompt prepended to conversation |
| `--model` | No | Override model. Accepts bare id (`gpt-5.3-codex`) or provider-qualified (`openai/gpt-4o`). Default: `gpt-5.5` |
| `--max-iterations` | No | Max tool call rounds before stopping |
| `--max-time` | No | Wall-clock timeout in seconds (exit code 2 on timeout). At the deadline the run is stopped: an in-flight model request is abandoned, a running bash command's process group is killed, and the unfinished request and the reason are written to the event log and pending request accounting flushed (waiting at most 5 s); blocking work already under way (a DNS lookup, a container teardown script, a swarm board write) may still finish before the process exits (#2168). A named session first saves its transcript, each tool call the stop cut short answered with an error saying so. Then a sub-agent launch the stop cancelled is rolled back: a container create still running is sent SIGTERM to remove what it made (see the create contract in docs/container-runtimes.md), and one already made is cleaned up. The process waits at most 5 s for this, as every harness exit does (#2173) |
| `--mode` | No | Operation mode: default one-shot, or `uds` for UDS event bus |
| `--socket` | No | Explicit socket path for `--mode uds` (default: auto-generated in tmpdir) |
| `--persist` | No | UDS mode only — keep agent alive when all clients disconnect (default: exit on last disconnect) |
| `--workflow` | No | UDS mode only — start workflow-driven prompt injection immediately |
| `--workflow-guards` | No | UDS mode only — enable workflow bash command guards; does not force prompt injection |
| `--no-workflow` | No | UDS mode only — explicitly disable workflow tool/state/prompt |
| `--parent-id` | No | UDS mode only — declares this agent's parent in the unit tree; stamped as `parent_id` on its `workflow_state` events. Set automatically by `spawn`; rarely passed by hand |
| `--effort` | No | Reasoning effort level (`none`/`low`/`medium`/`high`/`xhigh`/`max`), validated against the startup model's catalogue vocabulary (see [Reasoning effort is a per-model capability](../docs/runtime-models-providers.md#reasoning-effort-is-a-per-model-capability)); a level the model does not accept refuses to start, naming the ones it does. Overrides config and env var |
| `--disable-tool` | No | Disable a registered tool before the session starts (repeatable). Disabled tools remain in the descriptor catalogue for policy/UI callers, but are hidden from model-visible tool definitions and reject execution. Core names include `bash`, `read`, `write`, `edit`, `ls`, `grep`, `find`, `web_fetch`, `web_search`, `recall`, `spawn`, `agent_cmd`, `docs`, `workflow`; extension tools can be disabled by registered name. Unknown names warn on stderr but still start the agent. Every named tool is denied for the process lifetime in UDS (clients share the restricted set; `register_tools` cannot re-add a disabled name). Not a hard sandbox: disabling `write`/`edit` still leaves `bash` able to mutate the workspace. Child agents use spawn `disable_tools` / `read_only` instead (see [Subagents](docs/subagents.md)). |
| `--config` | No | Load exactly this config file (else `<base_dir>/config.json` with a trusted `./.quecto/config.json` overlay merged over it) |

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
| `prompt` | `message`, optional `id`, `streamingBehavior`, `images` | Send a user message. When agent is running, `streamingBehavior` (`"steer"` or `"followUp"`) is required. `images` attaches up to 8 PNG/JPEG/GIF/WebP images (`{"mimeType","data"}`, strict base64, readable header, ≤ 3.75 MiB decoded each); a bad image refuses the command ([details](docs/uds-protocol.md#image-attachments)) |
| `steer` | `message`, optional `id`, `images` | Interrupt after current tool, deliver this message (and its images) next |
| `follow_up` | `message`, optional `id`, `images` | Queue message (and its images) for after current run completes; if idle, run it immediately |
| `abort` | optional `id` | Cancel the current agent run |
| `get_state` | optional `id` | Return live supervision state, including model, streaming, accurate in-flight message count, effort, context limit, execution phase/current tool/recent progress, and workflow snapshot when enabled |
| `get_messages` | optional `count`, optional `before`, optional `agent_id`, optional `id` | Return stable committed transcript history (best used after the turn ends) as the newest bounded page; `count` requests an older-client newest slice, `before` pages backward, and `agent_id` targets a sub-agent. Responses include `messages`, `before`, and `hasMoreBefore` so older history is explicitly reachable. Oversized history entries are returned as recoverable summaries (`id`, role/tool metadata, preview `content`, `contentLength`, `collapsed: true`, `truncated: true`) and can be fetched deliberately with ranged `get_message`. |
| `get_message` | `messageId`, optional `offset`, optional `limit`, optional `agent_id`, optional `toolCallId`, optional `id` | Return one stable message by id. With `offset`/`limit`, returns a bounded content byte range plus `nextOffset`, `contentLength`, and `hasMoreContent` so oversized messages can be paged without exceeding the frame cap; `agent_id` targets a sub-agent; `toolCallId` recovers tool-call arguments instead of message content |
| `get_session_stats` | optional `id` | Return normalized token/cache usage, `costMicroUsd`, `cacheHitRatio`, and context occupancy (`contextTokens` / `maxContextTokens`); legacy float `cost` remains hidden from serialization |
| `list_models` | optional `id` | Return configured and built-in models from the runtime registry |
| `list_sessions` | optional `id` | Return every persisted session available for resume, newest first (`key`, `title`, `messageCount`, `updatedUnixSecs`) |
| `new_session` | optional `id` | Switch to a fresh user-chat session (idle only) |
| `resume_session` | `session`, optional `id` | Switch the active UDS conversation to a persisted session (a listed `chat-…` key, a `cli:<name>` key, or a bare CLI session name) |
| `set_model` | `model` or `provider`+`modelId`, optional `id` | Switch model at runtime |
| `set_effort` | `effort`, optional `id` | Set session reasoning effort (`none`/`low`/`medium`/`high`/`xhigh`/`max`, validated against the active model's catalogue vocabulary — the `effortLevels` `get_state` reports; a model with none refuses every level) |
| `get_tool_catalogue` / `list_tools` | optional `id` | Return the rich `ToolCatalogueEntry` snapshot for control/query clients in `data.tools` (bundled-native and UDS tools, policy/effective availability, source/owner/lifecycle/health) |
| `reload` | optional `id` | Force a provider/model config reload |
| `register_tools` | `tools` array, optional `id` | Register extension tools from a connected client |
| `unregister_tools` | `tools` array (names), optional `id` | Remove previously registered extension tools |
| `tool_result` | `toolCallId`, `content`, optional `isError` | Return result of an `execute_tool` request |
| `clear_history` | optional `id` | Clear conversation history, preserve system prompt |
| `rewind_to` | `messageId` (preferred) or `messageIndex`, optional `id` | Rewind conversation to a user-message boundary |
| `set_workflow_automation` | optional `id`, `autoContinue`, `completionNudge` | Toggle core workflow auto-continue/completion nudges for this UDS session |
| `get_subagents` | optional `id` | Return spawned subagents and live status. Each entry includes `readOnly` to identify observer sub-agents spawned with write/edit disabled |

**Events** (emitted as length-prefixed JSON frames; payload cap 8 MiB via `quecto-line-io`):

| Type | Description |
|---|---|
| `agent_start` | Agent begins processing a prompt |
| `agent_end` | Run finished; `messageRefs` identify this run's messages (legacy `messages` is empty after #1060 — resolve content via `get_message`) |
| `workflow_idle` | Post-turn drain found no further workflow continuation; optional `reason` (`exhausted` / `explicit_abort` / `completed`) |
| `token` | Incremental text token from streaming LLM |
| `turn_start` | New LLM call begins |
| `turn_end` | LLM call completed; `message` carries `messageRefs` / occupancy fields rather than full re-carried body after #1060 |
| `tool_execution_start` | Tool began executing (with `toolCallId`, `toolName`, `args`) |
| `tool_execution_end` | Tool finished (with `toolCallId`, `toolName`, `result`, `isError`) |
| `execute_tool` | Routed to extension client that registered the tool (not broadcast) |
| `tool_catalogue_changed` | Broadcast when the rich tool catalogue changes after runtime tool registration/unregistration (`changedTools`, `before`, `after`, `reason`) |
| `subagent_notification` | Passive child-agent completion/error/exit notification for UI visibility |
| `subagent_state_changed` | Broadcast replacement snapshot of spawned subagent statuses, including `readOnly` observer status |
| `subagent_messages_appended` | Child turn completed; `messageRefs` for the appended messages (`agent_id` set when forwarded onto a parent stream) |
| `workflow_state` | Broadcast when workflow mode/progress/template state changes |
| `response` | Response to a command (with `id`, `command`, `success`, optional `data`/`error`) |

### `quecto auth` — Manage API keys

```bash
# Pass token directly
quecto auth login --provider openai --token sk-proj-your-key

# No --token: the browser OAuth flow starts (same as --oauth) and blocks until the callback
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
| | `--token <key>` | Pass token directly (otherwise the OAuth browser flow starts; always `--token` from an agent) |
| | `--oauth` | Initiate OAuth browser-based login flow |
| | `--device-code` | Initiate device code flow for headless environments |
| `auth logout` | `--provider <name>` | Remove a stored credential |
| `auth status` | | List all stored credentials with status |

Credentials are stored in `<base_dir>/credentials.json` (`~/.quecto/credentials.json` unless `QUECTO_BASE_DIR` is set). The credential store takes priority over keys in `config.json`.

### `quecto status` — Check configuration

Shows the global config file, the repo-local overlay and whether it is trusted,
the effective workspace, model and effort, and API key status. Secret values
are redacted in status/debug output.

```bash
quecto status
```

### `quecto config` — Read and write configuration

```bash
quecto config get                                  # the effective (merged) document
quecto config get agents.defaults.model            # one value, as JSON
quecto config get --global tools.policy.entries    # one layer as written (--local for the overlay)
quecto config set agents.defaults.model '"openai-api/gpt-5.5"'   # writes ./.quecto/config.json
quecto config set --global agents.defaults.effort '"high"'       # writes ~/.quecto/config.json
quecto config unset agents.defaults.model          # remove a key (--global for the global file)
quecto config trust                                # approve ./.quecto/config.json's current content
```

Values are JSON; a bare word that is not valid JSON is taken as a string, so
`quecto config set agents.defaults.model openai-api/gpt-5.5` reads naturally (store the qualified `provider/model` id). `set` and
`unset` default to the repo-local overlay and refuse the global-only sections
(`providers`, `admission`) there; they also refuse to patch an overlay whose
current content is not trusted. `unset` of a key the layer does not set is an
error naming the layer (setting a key to `null` in the overlay would override
the global value, which is why removal is its own command). See
[discovery and precedence](#configuration-discovery-and-precedence) for the
merge rules and the writer's guarantees.

### `quecto admission-broker` — One host-wide inference broker

```bash
quecto config set --global admission '{"groups":{"shared":{"capacity":4,"reserve":1,"min_interval_ms":250,"queue_capacity":64,"queue_timeout_ms":120000,"attempt_timeout_ms":900000,"fallback_base_ms":2000,"max_cooldown_ms":600000}},"aliases":{"account":"shared"},"bindings":{"*":"account"}}'
quecto admission-broker install-service --dry-run   # the plan: unit path, daemon-reload, enable --now
quecto admission-broker install-service             # systemd user unit quecto-admission-broker.service
quecto admission-broker status                      # {"directory":…,"epoch":1,"journal_healthy":true,…}
quecto admission-broker reset                       # new epoch: roots re-register, children are respawned
quecto admission-broker uninstall-service --directory ~/.quecto/admission   # last, after `config unset --global admission` + restarting agents (needs --directory once the section is gone)
```

`status`, `reset`, `run`, `install-service` and `uninstall-service` address
the global file (`--config <file>` / `--directory <dir>` to pick another),
never the repo overlay; every output names the directory. Sessions started
before the section existed run unbounded until restarted. Runbook with
expected outputs and failure table: [inference-admission.md](docs/inference-admission.md)
(the same runbook the `docs` tool serves as `admission-broker`).

### `quecto container` — Standard container for a repository

```bash
quecto container init                 # bundle under .quecto/containers/standard + container_configs.standard in the trusted overlay
podman build -t quecto-box:local -f .quecto/containers/standard/Containerfile .quecto/containers/standard
quecto container status               # assets, entry, trust, image — ends with "ready: …" when all are in place
quecto container doctor               # the create's preflight, one ✓/✗ line per check, exit 0 when none failed
quecto container init --refresh       # after upgrading quecto, or to restore an edited script
quecto config unset --local container_configs.standard && rm -r .quecto/containers/standard   # rollback
```

Run from the repository root. Runbook with expected outputs and failure
table: [container-runtimes.md](../docs/container-runtimes.md#the-standard-container-quecto-container-init-2024-s4e)
(the `docs` tool serves it as `container-runtime`).

### First run — zero config

quecto needs no setup step. With no config file it runs on defaults; supply a
key via `quecto auth login` or `QUECTO_*` env vars. A config file is optional —
when present it's read from `~/.quecto/config.json`, with a trusted
`./.quecto/config.json` in the working directory merged over it (see
[discovery and precedence](#configuration-discovery-and-precedence)), and the
workspace is created on demand:

```
~/.quecto/
  config.json     # optional — defaults apply when absent
  workspace/       # agent working directory
```

### Configuration discovery and precedence

Every command that loads configuration (`status`, `agent`, `admission-broker`,
`config get --effective`, and the REPL's `status`) reads its configuration from
these layers (#1966, #2024):

1. `--config <path>` — an explicit selection replaces everything; the file must
   exist. Nothing else is read.
2. `<base_dir>/config.json` — the global file (`~/.quecto/config.json`, or
   `$QUECTO_BASE_DIR/config.json`). Absent means defaults apply.
3. `./.quecto/config.json` — the **repo-local overlay** in the process working
   directory, merged *over* the global file. Only the working directory itself
   is probed, never its parents. It is applied only when trusted (below); an
   absent overlay is simply not there.

The overlay is merged section by section, so a repository needs at most a
small file and never duplicates your secrets or policy:

| Section | Merge |
|---|---|
| `agents.defaults` | field-wise — an overlay field replaces the global field, other fields stay |
| `tools.web` | field-wise per engine (`brave`, `duckduckgo`, `fetch`) |
| `tools.grep` | field-wise per section (`relevance`, `log`) |
| `tools.policy.entries` | entry-wise — an overlay entry replaces the global entry of the same stable id |
| `container_configs` | entry-wise; an overlay entry with `"default": true` un-defaults every global entry |
| `workflow` | field-wise (`templates` replaced whole) |
| `providers`, `admission` | **global-only** — an overlay carrying either is refused with an error naming the key |
| anything else | replaced whole (unknown keys pass through both files) |

The global file must be a valid configuration on its own, the overlay a
valid layer (an overlay may add a container config without claiming the
default), and the merge a valid configuration. A trusted overlay that is invalid JSON, a directory, a dangling
symlink or unreadable is an error naming the path, never a silent fallback.

**Trust.** A checked-out project's overlay is applied only once its exact
content has been approved: the approval is recorded by canonical path and
SHA-256 in `<base_dir>/config-overlay-trust.json`. Approve it explicitly with
`quecto config trust` from the project directory (an agent or a script can do
this without a terminal), or answer the `[y/N]` prompt a one-shot
`quecto agent` run offers from an interactive terminal — it prints the
overlay (the first 4 KiB) before asking, and the answer is recorded only after
the same checks `quecto config trust` applies. One content per
path is trusted: editing the overlay by hand revokes its trust until it is
approved again, and reverting to an earlier content does not restore it;
`quecto config set` records trust for what it writes. An untrusted overlay is
reported (which file, its hash, and the command to run — or, when
`quecto config trust` would refuse it, why) and **not** applied, so nothing in
a repository you have not reviewed can change your agents' defaults. The
overlay must be a regular file in a regular `.quecto` directory: a symbolic
link at `.quecto/config.json` *or* at `.quecto` itself is refused whatever it
points at (trust is keyed by the file's identity, which a link would borrow
from its target), and `quecto config set` never writes through one. `quecto status` prints both files and the
overlay's trust state (`trusted`, `untrusted`, `refused` or `none`):

```
$ quecto status
quecto Status
  Config:    /home/me/.quecto/config.json
  Overlay:   /home/me/src/app/.quecto/config.json (trusted)
  Workspace: /home/me/.quecto/workspace
  Model:     openai-api/gpt-5.5
  Effort:    default
  ...
```

**Migration.** Until #2024 a `./config.json` directly in the working directory
*replaced* the global file. That selection is retired: such a file is no longer
loaded, and `quecto status` warns while one that looks like a quecto
configuration (a JSON object with a known section) exists — an unrelated
`config.json` at a project root is left alone. Move its repo-specific
settings to `./.quecto/config.json` (`quecto config set …` writes them for
you) and any `providers` or `admission` section to the global file.

**Reading and writing.** `quecto config get [<dotted.path>]` prints the
effective value (`--global` or `--local` print one layer as written).
Secret-shaped values — keys named `api_key`, `apiKey`, `token`, `secret`,
`password`, or ending in `_key`/`_token` — print as `"<redacted>"` unless you
pass `--show-secrets`, because agents run this command and its output lands
in model context and transcripts. `quecto config set <dotted.path>
<json-value>` writes the repo-local overlay (`--global` writes the global
file). The writer patches the JSON *document*: only the addressed key's value
changes, every other key and the key order are kept, and the result is
validated before anything is written — the file on its own and the effective
configuration this directory would then load (global merged with the trusted
overlay), so a change that is fine in one file but breaks the merge (an overlay
container config that leaves no default) is refused rather than bricking the
next run. The write is atomic (tmp + fsync + rename) and serialised per file
on a lock under `<base_dir>/locks/` (never beside the file, so a refused
`config set` in a clean checkout creates nothing there — not even `.quecto/`),
so concurrent `config set`s never lose each other's keys. It does normalise
layout: the file comes back as pretty-printed JSON in its own indentation
(two spaces for a new or compact file) with LF line endings and one trailing
newline, and numbers are re-rendered — a file already in that layout changes
only on the touched line. A refused value leaves the file byte-identical.
`set_tool_policy … persist` writes through the same path into the base file;
an entry the trusted overlay already defines is refused (it would be shadowed
on the next reload) with the `quecto config set` command to run instead.

**Default model and effort per repository (#2024 S2).** `agents.defaults.model`
and `agents.defaults.effort` in the overlay pin what every agent started in
that directory starts on — `quecto agent`, `quecto-tui`, a spawned local
child — while a sibling repository keeps the global default. Pin one with
`quecto config set agents.defaults.model '"provider/model"'`, from a running
session with `set_model … "persist":"local"` (`"global"` for the run's global
layer: the global file, or the `--config` file when one was given; the same
for `set_effort`), or in `quecto-tui`'s `/model` selector (Tab cycles
*use for this session* / *use and pin as this repo's default* / *use and pin as
the global default*). Every path goes through the writer above and is refused
with the session unchanged when the overlay is untrusted or a symbolic link,
when the run has an explicit `--config`, or for a bare model id (only
`provider/model` is recorded). Verify with
`quecto config get --effective agents.defaults.model` and `quecto status`; roll
back with `quecto config unset agents.defaults.model`. `quecto agent --model`
/ `--effort` and `quecto-tui --model` / `--effort` choose for one run without
writing anything.

A running agent re-reads the base file and the overlay (and, while an
overlay exists, the trust record) before the next turn, `set_model` or a
forced `reload`, so `quecto config trust`, a `config set`, or creating or
removing the overlay is picked up without a restart. What a reload changes
in the running loop is the providers and the tool policy; a new
`agents.defaults.model` or `effort` in the files applies to the next run (use
`set_model`/`set_effort` for the current one). A `config set` writes the
overlay and then its trust record, so a reload that lands between the two
applies the global file alone for that one turn and reports the overlay
untrusted; the next reload applies it. A missing base file never triggers a
reload: the last-good runtime is kept. `quecto-tui` sessions never prompt for
trust; run `quecto config trust` in the project directory and the next turn
picks it up.

**Container spawns read the effective configuration.** A `spawn` with
`container: true` (or a named `container_config`) selects from the
`container_configs` a run in the agent's working directory would load — the
global file with the trusted overlay merged entry-wise, resolved fresh at
every spawn — so a repository binds itself to a container config with
`quecto config set --local container_configs.<name> '{"default":true,…}'`
and rolls back with `quecto config unset --local container_configs.<name>`.
A repository's `standard` entry (written by `quecto container init`) is its
default by rule: `container: true` selects it whatever the global file or
another overlay entry labels (#2035).
There is no separate container trust record or `[y/N]` prompt: `quecto
config trust` is the one approval, and an untrusted overlay contributes
nothing — `container: true` is then refused with the diagnostic in the tool
result, a named `container_config` launches from the global set and carries
it (the spawn also prints it to stderr, as `status` does). The binding
applies only to runs started without `--config` and never inside a
container child, which is started with the global file. The pre-#2024
`container-config-trust.json` is not read; approve such an overlay once with
`quecto config trust`. See [Container runtimes](../docs/container-runtimes.md).

Subagents: a locally spawned subagent inherits the parent's working directory
and performs its own discovery there — the same global file and, when trusted,
the same overlay. A parent's explicit `--config` is *not* forwarded to local
children (pass `config` in the spawn call, or set
`QUECTO_RUNTIME_CONFIG_PATH`, to pin a child's file). Container subagents are
handed the parent's selected base file explicitly. `QUECTO_RUNTIME_CONFIG_PATH` is a
child-launch mechanism only: it is the `--config` given to spawned children
when neither the spawn call nor (for containers) the parent supplies one; it
does not affect the launching process's own selection above.

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

Config file: `~/.quecto/config.json`, with a trusted repo-local
`./.quecto/config.json` overlay merged over it (see
[discovery and precedence](#configuration-discovery-and-precedence)). Prefer
`quecto config set` over hand edits: it patches one key and leaves the rest of
the file untouched.

```json
{
  "agents": {
    "defaults": {
      "model": "gpt-5.5",
      "workspace": "~/Documents/quecto-workspace",
      "max_tokens": 8192,
      "max_tool_iterations": 999999,
      "max_session_messages": 200,
      "max_context_tokens": 300000,
      "context_high_tokens": 256000,
      "context_low_tokens": 70000,
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
- `openai-oauth/...` uses the stored `quecto auth login --provider openai --oauth` credential.
- `anthropic-api/...` uses `providers.anthropic.api_key` / `ANTHROPIC_API_KEY`.
- `anthropic-oauth/...` uses the stored `quecto auth login --provider anthropic --oauth` credential.

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

Supported provider fields: `api` (`openai-completions` or `anthropic-messages`), `baseUrl`/`apiBase`, `auth`, `authHeader`, `allowRemoteHttp`, and `models`. API keys support `$ENV` and `${ENV}` interpolation. Supported model fields include `id`, `name`, `reasoning`, `input`, `contextWindow`, `maxTokens`, and `cost` (`input`, `output`, `cacheRead`, `cacheWrite`). A model is sent images only when its `input` includes `"image"`; otherwise each image becomes a `[image not sent: <model> takes no image input]` marker (#2421). An `overrides` entry may set `input` to turn images off or on for a built-in model; a record without `input` keeps the built-in one, so a custom provider of your own (for example one listing `claude-opus-5` on `anthropic-messages`) must set `"input": ["text", "image"]` to send images. A bare model id reads the router's first provider, and takes no image when that provider does not list it.

To use an OAuth-backed registry provider, first run `quecto auth login --provider openai --oauth` or `quecto auth login --provider anthropic --oauth` (a human at a terminal; an agent stores an API key with `--token <key>` instead), then select the registry provider key (for example `/model anthropic-oauth/claude-opus-4-8`). The `/model` selector shows `[apiKey]` or `[oauth]` so the billing/auth mode is visible before selection.

`providers.openai_compatible.endpoints` remains supported for OpenAI-compatible API-key endpoints, but `models.json` is preferred when you want those models to appear in `/model`.

### Exec behaviour

- `bash` commands run natively in the workspace root, each in a fresh shell (`cd` and `export` do not carry over between calls). The shell is chosen once per process (#2195). An allowlisted, installed bash named by `$SHELL` is used; otherwise the first installed of `/bin/bash`, `/usr/bin/bash` and `/usr/local/bin/bash`; otherwise the allowlisted `$SHELL`; otherwise `/bin/sh`. Without bash, the tool's description says bash-only syntax may not work.
- exec child processes clear the ambient environment and then explicitly receive the current process environment (or test-provided overrides).
- `Sandbox::validate_command` rejects a denylist of destructive commands (e.g. `rm -rf /`, recursive `chown root`) before execution.
- There is no built-in process/network/resource isolation. For untrusted workloads, run Quecto inside a container (or other OS-level sandbox), which bounds filesystem, network, and resource access for the whole process.

### Swarm

`swarm` is registered but refuses to work outside a recognised Docker/Podman swarm container. Members coordinate through structured board ops (`swarm {"op":"claim","task_id":3}`), which the in-process Rust board serves (ADR-0030). See [Swarm coordination](docs/swarm.md) for setup, coordination, membership and verification.

Workflow is unavailable for swarm coordinators and workers; workflow flags, guards and bound specs are rejected on their launches. Agents can read the embedded manual with `docs {"name":"swarm"}`, which gives every board op with a runnable example. Members use the separately configured `bash` tool for Git, tests, computation and external commands.

For progress, ask the coordinator for `swarm {"op":"summary"}` and retrieve its report through `agent_cmd.get_messages`. After completion, `summary`, `events`, `usage` and ordinary artifact export remain available, but every board op (`inbox` and `ack` included) is refused unless the supervisor resumes the run. There is no dedicated swarm dashboard or public UDS board API yet. See [inspection and results](docs/swarm.md#inspection-and-results-for-users-and-master-agents).

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
| `QUECTO_SWARM_MAX_CONTEXT_TOKENS` | `agents.defaults.swarm_max_context_tokens` (a swarm member's ceiling; `0` is refused) |
| `QUECTO_CONTEXT_HIGH_TOKENS` | `agents.defaults.context_high_tokens` (the watermark's high mark, default `256000`) |
| `QUECTO_CONTEXT_LOW_TOKENS` | `agents.defaults.context_low_tokens` (the watermark's low mark, default `70000`; at most 0.9 × the high mark) |
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
| `read` | Read file contents (text or image). Text: 2000-line / 50KB truncation with offset/limit pagination; repeated unchanged same-path/same-scope text reads may return an unchanged marker unless `force: true` is passed; once a watermark cut archives a read's result, or the emergency ladder stubs or drops it, the next read of that file returns its content (#2348, #2414). Images (jpg/png/gif/webp): base64-encoded. Magic-byte MIME detection. A missing file, or a link to one, is a refusal that names it (#2193). Every failure is a refusal that names the path, gives a plain reason and a next step, with no `tool error` prefix (#2189): a directory (list it with `ls`), a missing file (where it was looked for, and the directory to list), a path through a file, a symbolic-link loop (the link and where it points; a real cycle is told from an acyclic chain of more than 40 links, which the system also refuses, by reading the links without following them), permission denied, a bad `offset`/`limit`; any other system error keeps its own text. A file that starts with a UTF-16 byte-order mark is named as UTF-16 with the `iconv -f UTF-16 -t UTF-8` conversion; one that is clearly binary (a NUL byte, or an executable, archive or compressed format's leading bytes) is named as binary with `file`/`xxd` to inspect it; other non-UTF-8 text also gets an `iconv` hint |
| `write` | Create or overwrite a file (auto-creates parent directories). A file is replaced whole or not at all (#2243): the content goes to a hidden, unpredictably named temporary file in the target's own directory (created exclusively, not through a link, with the target's mode), is fsynced and renamed over the target, and the directory is fsynced; on any error the temporary file is removed and the target is untouched. A symbolic link is kept and its target written; a link to nothing has its final target created the same way, from a temporary file beside it, so a failed write leaves no partial target. The temporary name keeps at most 100 bytes of the file's name, so a long name still fits. Where a rename cannot stand in for a plain write the file is written in place (not atomically: a failure may leave it cut short or a mix of old and new text) and the result says so in a note: a file with hard links; a writable file in a directory that takes no new file; a rename the system refuses (a single-file bind mount or other mount point, `EBUSY`/`EXDEV`; another user's file in a sticky directory, a security policy such as SELinux or AppArmor, or an append-only directory, `EPERM`/`EACCES`), where a temporary file that could not be removed is named in the note; and an owner or group that a new file cannot be given (the whole owner is given back only by a privileged writer, the group alone when the writer is in it), so a file keeps its owner, group and mode. A full disk or quota while the temporary file is written leaves the file as it was and says to free space and retry (no fall back to an in-place write). A file removed while it is written (its handle has no links left) is refused as removed, with nothing written; a temporary file that could not be removed after a failure is named so it can be deleted; a write that stops on an internal fault says that what it left is not known. A new file that cannot be created in its directory points at the directory (`ls -ld`). A file the writer may not write is refused before anything is created; the file is opened without blocking and must be the regular file that was looked at, so a FIFO or a file swapped in meanwhile is refused. A rename makes a new inode, so POSIX ACLs, extended attributes and SELinux labels are not carried over (a write in place keeps them). Races left: a file created at a missing name while the new one is written is replaced by it, and a swap after the file is opened and before the rename is replaced |
| `edit` | Replace text in a file. Two-stage exact→fuzzy matching, CRLF/BOM preservation, no-op detection, LCS-based unified diff output (at most 4 KB). A refusal names the file and says what to do next (#2193): an `oldText` that matches more than once gives the true number of matches (overlapping ones count) and the lines of the first three; a missing file points to `write`; a directory or a file over 1 MiB is named as such, and one that is not UTF-8 text (binary) is refused in the words `read` uses (#2166); and when nothing matches but a match exists with other indentation, the lines it matches at (all of them, when more than one), the first line whose indentation differs and both indentations (tabs or spaces) are given. A changed line longer than 240 bytes is shown as a window around its own change (60 bytes either side, a long change shown by its two ends, `…` where text is left out) with a note giving its column, kept with its line when the diff is cut: a removed line is paired with the added line that shares the most text with it at its start and end, and each is windowed on their difference (a pair that differs only in its line break says so); a line with no partner is windowed on its part of the edit, or shows its two ends when wholly added or removed; any other long line shows its start (#2194). The file is written as `write` writes it (#2243): replaced whole or not at all, so a failed write is refused by name and leaves the file as it was; only a file written in place (see `write`) may be cut short, and the refusal then says how to restore it; line numbers in refusals count the file's own line breaks, as `read` shows them. An edit changes only the matched span (#2242): matching runs on normalised text (BOM stripped; `\r\n` and a lone `\r` read as `\n`), and the replacement is spliced into the file's own bytes, so everything outside the span (the BOM, CRLF or mixed line endings, a lone `\r` in a captured progress log) is written back byte-for-byte. each of `newText`'s line breaks is written with the ending of the line break it replaces (the n-th break of `newText` takes that of the n-th break of the matched span), and breaks beyond those take the ending of the last replaced line (of the line it is on, for a match within one line), so an edit keeps each line's own ending even in a mixed file; an edit that would change only the form of a line break is refused as making no change; a lone `\r` inside the matched span is replaced with the rest of it. Known edge: a lone `\r` just before the match is kept, so deleting the text between it and a following line break joins the two into one `\r\n` (`a\rX\nb`, `X` → `` gives `a\r\nb`). The diff's line numbers count the file's own line breaks |
| `ls` | List directory contents. Case-insensitive sort, `/` suffix for directories, configurable limit (default 500, max 5000), 50KB output cap. A longer listing is the sorted start of the whole directory (read in full, keeping only `offset + limit` entries), with a note giving the total and the next `offset` (max 100000) (#2188). Every call reads the whole directory, with no deadline (a dropped call stops at the next entry), so a huge directory is slow; a `limit` that does not round to at least 1, an `offset` below 0 or above 100000, or either one not a number, is refused (a `limit` above 5000 is 5000) Listing a file says to read it, a missing directory names where it was looked for and the directory to list, and a link loop or permission denied is named, each as a refusal with no `tool error` prefix (#2189) |
| `grep` | Search file contents with ripgrep, the one content search agents are told to use (not `rg`/`grep`/`git grep` in bash). Several patterns, globs (`!` excludes) and file types, case-insensitive, literal, whole-word, multiline, per-file cap; `output` = `content` (lines with context from a file cache), `files` (each matching file once) or `count` (matches per file, busiest first); skips `.git` internals; partial results before an rg error or past the read cap come with a notice; 100-match (or file) / 50KB limit. A line over 500 bytes is cut to 500 bytes: a matching line to windows around its matches (at most 4; close matches share one), any other line from its start, with `…[N bytes]…` marking what is left out and the line's size and first match's byte offset after it (#2201). Offsets and byte counts are in the line's raw bytes (as `cut -b` counts them), also on a line that is not UTF-8, which is shown with U+FFFD for its invalid bytes. A directory search skips binary files (rg's rule: a NUL byte); when it finds nothing, a second rg run lists the files holding a match with `--binary` (paths only, at most 2 s and 200 KB of output), each of which the search skipped, and the result counts them and names the first three in path order (quoted; "at least" with examples when the check was cut short or rg reported an error); a search that found matches is not checked. A directory search also stops reading a file at a NUL byte found after a match: content output names such files, while count output leaves them out. A file named as `path` is searched whole, binary or not (#2202) |
| `find` | Find files and directories by glob pattern with fd. Paths are relative to the workspace, as `grep` prints them (absolute outside it), so they pass to `read`/`edit` unchanged (#2203). Respects nested `.gitignore` files and lists hidden files, but skips VCS internals (`.git`, `.hg`, `.svn`, `.jj`) below the search root; passing one as `path` searches it, and an empty result says where the VCS metadata lives (a worktree's real git directory when `.git` is a `gitdir:` file naming one) (#2199). `type: "f"` / `"d"` lists only files or directories, a symlink counting as what it points at (and never shown with `/`); with a type, fd's results are filtered as they arrive and fd is stopped once `limit + 1` are kept. A pattern ending in `/` means directories (#2200). fd's output is read NUL-separated and names are looked up by their bytes; a control character in a name (a newline) is shown escaped and a non-UTF-8 byte as U+FFFD, so each entry is one line. The output cap counts only kept entries. A `limit` that does not round to at least 1 is refused, as grep refuses it. Path-segment patterns via `--full-path`, configurable limit (default 1000), 50KB output cap |
| `swarm` | Container-only coordination of a bounded pool of agents: durable tasks, claims, messages, evidence and file reservations on a SQLite board served in-process. `op=create` starts a run; each board method is a structured op of the same name and arguments (`{"op":"claim","task_id":3}`, `{"op":"submit",...}`, `{"op":"send",...}`), answering its JSON result; `summary`, `events`, `usage`, `pause`, `reconcile` and `cancel_run` are the harness's own ops. See [Swarm coordination](docs/swarm.md) |
| `recall` | Retrieve a spilled tool output by its spill ID (e.g. `turn20:bash:0`). Use `recall("list")` for the full index. An unknown id points to `recall("list")`; an id that is a file path (such as the file bash saved a long output to) is sent to `read`, with the call to make (#2215), or, when the file is over `read`'s 10 MiB cap, to bounded paging with bash (`sed -n '1,200p'`, `tail -n 200`); with no file there to size, both are given |
| `spawn` | Spawn a background UDS-mode subagent. Save the returned UUID for all child-targeted `agent_cmd` calls; spawn `agent_id` is a UI label only. `read_only: true` disables write/edit, but is not a sandbox: `bash` can still mutate the workspace |
| `agent_cmd` | Send commands to spawned UDS subagents: `swarm_control`, `get_report`, `prompt`, `steer`, `follow_up`, `abort`, `kill`, `get_state`, `get_messages` (omit/null `count` and `before` for the latest substantive assistant message on the first call, then unread deltas; no new content returns unchanged; the cursor advances only on successful delivery to the parent model; explicit `count`/`before` request cursor-neutral history pages; `before` pages backward), `get_session_stats`, `get_subagents`, `get_subagents_all`, `get_containers` (the environments an existing-mode join admits plus this session's own not stopped, compact, in ref order, capped at 20 with counts; all=true also lists stopped ones and other sessions' that can't be joined), `get_container_configs` (the container configs a spawn can select for this checkout, with each entry's source and repository), `kill_container`, `set_model`, `set_effort`, `clear_history`. Prefer spawn → end turn → passive completion note → `get_messages`. Do not poll `get_subagents` / `get_subagents_all` or sleep as a wait loop. |

For `agent_cmd` command `get_subagents_all`, pass `agent_id` as `*` to list the current parent agent's tracked subagent registry instead of targeting a child agent.

| `web_search` | Optional: search the web via Brave Search or DuckDuckGo when `tools.web.brave.enabled` or `tools.web.duckduckgo.enabled` is true (Brave titles and snippets are read as plain text: highlight tags stripped and HTML entities decoded before they are cut to length, #2211) |
| `web_fetch` | Optional: fetch a URL and return readable text when `tools.web.fetch.enabled` is true (HTML is made readable unless `raw: true`, and a page that marks its main content with one `<main>`, `role="main"` element or `<article>` holding at least a third of its text, however much more, is read from that element alone, under its title and a one-line note giving the bytes of page text dropped (a landmark that holds all the text drops nothing, and the whole page is returned with no note), unless `main_only: false` (#2225); adjacent block elements, links, buttons, labels and table cells are kept apart in the readable text (a link or cell after another is spaced whatever it ends in, such as `50%` or `«`), except inside `<pre>`, `<code>`, `<kbd>` and `<samp>` (code markup left open ends at the next block or cell outside `<pre>`), after code punctuation such as `.` or `::` and within text in a script written without spaces between words (only Latin, Greek and Cyrillic letters and ASCII digits are spaced from a link straight after them); other text such as JSON, markdown or CSV comes back as served; binary content is named, not shown (#2165)). Only public destinations are fetched (#1942): the URL's address (in any spelling, including IPv4-mapped, NAT64, 6to4 and Teredo IPv6 forms), every address its name resolves to (the connection goes only to the checked addresses), and every redirect hop must be globally routable unicast; private, loopback, link-local (cloud metadata), shared, documentation and reserved addresses are refused with the reason. web_fetch never uses a proxy: it connects directly to the checked address and ignores `HTTP_PROXY`, `HTTPS_PROXY` and `ALL_PROXY`, since a proxy would resolve and reach names unchecked (a warning is logged once when any is set, and while one is set a failed connection says `(web_fetch ignores HTTP(S)_PROXY; see #1942)`). A failed connection or body read gives its cause, not only `error sending request`: the error's cause chain (at most 512 bytes, the proxy note included), led by a plain reason where one is known (connection refused, DNS lookup failed, TLS certificate not accepted, TLS handshake failed, timed out) and, when it failed on a redirect hop (after any redirect, even one back to the URL asked for), by that hop's URL (#2209). URLs in failures and refusals are shown by scheme, host, port and path only (no userinfo, fragment or query, which shows as `?…`), at most 128 bytes, and the URL asked for comes after the causes; a redirect hop, whose URL the server chose, is shown with its first path segment only, the rest as `/…`, so that a token in its path is never shown (the URL asked for, the agent's own input, keeps its path). It has its own client and connection pool, not the providers' shared client: the adapter builds it from an owned recipe (the shared connect timeout and TLS roots) with its own resolver, redirect handling and no proxy, so no Unix socket or other transport can be handed to it. Redirects (301, 302, 303, 307 and 308, at most ten) are followed only when the response carries exactly one valid `Location`, all within the one 10-second deadline; any other redirect is refused (`Redirect refused: ...`), including a 301/302 with no `Location`, which before #1942 came back as the plain 3xx status. Credentials from the URL's userinfo follow same-host hops only. `localhost`, `localhost.localdomain`, `*.localhost` and the cloud metadata names are refused up front as a courtesy. NAT64: the well-known prefix `64:ff9b::/96` is always judged by the IPv4 address it embeds, and a network-specific prefix is discovered once per client (RFC 7050: `ipv4only.arpa` looked up through the same resolver, at most 2 seconds and within the fetch deadline; no answer means no prefix) so that an address under it is judged by the IPv4 address the translator reaches (RFC 6052). A translator that no DNS64 advertises cannot be discovered, and an address under its prefix is judged as IPv6. |
| `workflow` | UDS-only template-based development workflow (status, list_templates, select_template, check, uncheck, skip, reset, set_issue, clear_issue, check_guards with command). Available by default in UDS as a dormant tool unless `--no-workflow`; `--workflow` starts prompt-driven mode immediately. See [Workflow docs](docs/workflow.md) |

Filesystem tools (`read`, `write`, `edit`, `ls`) run on async `tokio::fs` adapters.

## Security

Quecto provides limited in-process policy checks, not isolation; real isolation is the deployment's job (run it in a container). Understand the split before exposing it to untrusted input:

- **Filesystem tools are not confined**: `read`/`write`/`edit`/`ls` can access any path the Quecto process user can access. Real isolation must come from the deployment/container.
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
- ChatGPT OAuth requests also send that key as the `session_id` header, as the official Codex client does (#2162)
- Encrypted reasoning (#2162): each `reasoning` item the service returns with `encrypted_content` is kept on its assistant message and in the saved session. At most 1 MiB is kept per response. On the next request it is sent back just before the call (or reply text) it led to, in the order it was produced, and without its id. It is replayed only to its origin: the same endpoint, ChatGPT account (kept by digest) and model. It is never shown, and a reply carrying only encrypted reasoning still counts as empty. A model that returns no reasoning items (`gpt-6-sol` below effort high) sends none
- Orphaned tool call pair repair: mismatched `function_call`/`function_call_output` pairs (from context pruning or mid-turn interruption) are detected and dropped before sending to the API
- Parallel tool calls enabled (they run at once only when every call may overlap; see Parallel tool calls)

### OAuth auto-refresh

OAuth-backed providers are wrapped in `RefreshableProvider` so that expired tokens are automatically refreshed mid-session on 401. The decorator intercepts auth errors, refreshes the token via the credential store, rebuilds the inner provider with the new token, and retries the request once.

API key resolution order: credential store (`quecto auth login`) > environment variable > config file.

## Development workflow

Quecto development uses the repository-local Quecto workflow checklist:

Pure-move refactors (for example file extractions, renames, or byte-identical moves) should ship in their own PR so reviewers can distinguish structural movement from behavior changes. That standalone refactor PR may land before or after the behavioral change that motivates it.

1 - Install/check local quality hooks
2 - Update Scenarios / Add new features
3 - Write/update unit tests (run a quick smoke check; full CI runs after `merge-requested`)
4 - Ensure new/modified tests FAIL (RED) — quick targeted run only, not full suite
5 - Despatch three BDD review finders (Gherkin discipline, Falsifiability, Coverage)
6 - Implement code (GREEN)
7 - Refactor (perf, security, clean arch)
8 - Ensure tests still pass (GREEN)
9 - Bump semver for every changed crate and sync crate-specific version assertions
10 - Commit
11 - Push through the fast pre-push gate
12 - Create PR
13 - Despatch narrow parallel review finders, verify adversarially, post one review
14 - Fix all valid review concerns
15 - Push changes to remote
16 - Reply to the reviewers comments on the PR and mark resolved (use graphql)
17 - Verify the PR meets every issue acceptance criterion
18 - Request authoritative CI and report the PR (do not merge)
19 - Clean up sub agents

## Quality gates

| Gate | Command |
|---|---|
| Quality scripts | `scripts/check-quality.sh`, `scripts/check-bdd-quality.sh` |
| Format | `cargo fmt --check` |
| Lint | `cargo clippy -p quecto -- -D warnings` (zero warnings) |
| Unit tests | `cargo test --workspace --no-fail-fast --features quecto-agentic-harness/test-support --bins --lib 2>&1 \| scripts/test-filter.sh` |
| Architecture | `cargo test --workspace --no-fail-fast --features quecto-agentic-harness/test-support --bins --test architecture 2>&1 \| scripts/test-filter.sh` |
| BDD (sharded) | See [Sharded BDD](#sharded-bdd-24-way-parallel) below |

All test commands pipe through `scripts/test-filter.sh` which strips the per-test `... ok` noise and shows only:
- **Summary totals** (passed/failed counts)
- **Failure details** (test name, file:line, assertion message, panic reason)
- **BDD failures** with Feature/Scenario context

`--no-fail-fast` ensures all failures are reported in a single run, not just the first.

Two-tier local hooks: pre-commit performs lightweight staged-file hygiene and formatting; pre-push runs fast quality rules, changed-package strict Clippy, and architecture/repository invariants. Full tests, BDD, coverage, dependency policy, and mock E2E run in authoritative CI only after the `merge-requested` label is applied; the same label starts advisory mutation testing of the PR's changed lines (see CONTRIBUTIONS.md). A subsequent push removes that label. Install via `scripts/install-hooks.sh`.

### Sharded BDD (24-way parallel)

Non-real-LLM (fast, no API key needed):
```bash
(for i in $(seq 0 23); do
  (timeout 12m env QUECTO_BDD_SHARD_INDEX=$i QUECTO_BDD_SHARD_TOTAL=24 cargo test --workspace --no-fail-fast --features quecto-agentic-harness/test-support --bins --test bdd 2>&1 | scripts/test-filter.sh) &
done
wait)
```

Provider smoke (paid, opt-in, minimal live request):
```bash
QUECTO_PROVIDER_SMOKE=1 QUECTO_TAG=provider-smoke cargo test --workspace --no-fail-fast --features quecto-agentic-harness/test-support --bins --test bdd 2>&1 | scripts/test-filter.sh
```

Provider smoke runs only provider-specific scenarios with available credentials: OpenAI uses `OPENAI_API_KEY`, Anthropic uses `ANTHROPIC_API_KEY`, and Codex uses an existing OpenAI OAuth credential in the `quecto` credential store. Missing provider credentials filter out that provider's smoke scenario without failing unrelated smoke checks.

Legacy live behavioral suites are tagged `@manual-real-llm` and still gated by `QUECTO_REAL_LLM=1`, but behavioral e2e coverage should prefer mocked provider responses so normal test runs do not incur provider costs.

### Running individual features or scenarios

To debug a single scenario, add a temporary tag (e.g. `@focus`) to the scenario in the `.feature` file, then run:
```bash
QUECTO_TAG=focus cargo test --workspace --no-fail-fast --features quecto-agentic-harness/test-support --bins --test bdd 2>&1 | scripts/test-filter.sh
```
Remove the tag before committing.

## Testing

```bash
# Core suite (no real provider calls)
cargo test --workspace --features quecto-agentic-harness/test-support --bins --test bdd

# Core suite (24-way sharded, fastest local full run; the binary is built once
# and run per shard. CI uses --shards 8 on 4-vcpu runners.)
bash scripts/run-bdd-shards.sh --suite non-real-bdd --shards 24 --timeout 12m


# Mocked e2e suite (free, deterministic, authoritative CI lane — no API key)
bash scripts/run-bdd-shards.sh --suite mock-llm-bdd --shards 24 --timeout 12m --tag mock-llm

# Provider smoke subset (paid, opt-in; filters providers without credentials)
QUECTO_PROVIDER_SMOKE=1 QUECTO_TAG=provider-smoke cargo test --workspace --no-fail-fast --features quecto-agentic-harness/test-support --bins --test bdd

# Live Real-LLM full suite (paid, manual/on-demand — needs OPENAI_API_KEY in .env)
bash scripts/run-bdd-shards.sh --suite real-llm-bdd --shards 24 --timeout 12m --tag manual-real-llm --real-llm
```

The e2e suite exists in two parallel lanes that assert the same behaviours:
- **`@mock-llm` (default, free):** WireMock-backed deterministic coverage under `tests/features/e2e_mock_llm*.feature` plus the full `@manual-real-llm` behavioral mirror. Makes zero paid provider calls and passes with no API key. This runs in authoritative CI after `merge-requested` is applied.
- **`@manual-real-llm` (manual, paid):** the retired live behavioral suite under `tests/features/e2e_real_llm*.feature`, retained for occasional exploratory end-to-end validation. Run it on demand with the command above.

Contributor rules for the live/mock e2e split:
- Keep behavioral scenarios dual-tagged with `@manual-real-llm @mock-llm` when they should run in both lanes.
- Mock only external provider HTTP responses in the `@mock-llm` lane. Do not synthesize app-level events such as UDS `agent_end`, `token`, `workflow_state`, or `get_state` responses.
- Preserve scenario inputs used by the live/manual lane. If mock routing needs a test-only provider alias, keep it behind `test-support`, `QUECTO_TAG=mock-llm`, and loopback mock URLs rather than editing live scenario text.
- Put provider protocol edge cases in provider/unit tests. BDD e2e mocks should stay focused on application behavior: tools run, files change, sessions persist, REPL/UDS/subprocess output appears, and workflow events are produced by real app paths.
- When a scenario asserts tool or workflow behavior, the mocked provider must return the relevant tool-call response(s) before the final text marker. Do not make those scenarios pass by returning only the marker text.
- For UDS workflow scenarios, use the real multi-client socket path when asserting broadcast-only events. The test harness should read the socket while the run is active to avoid backpressure on large workflow event streams.
- Keep `@provider-smoke` tiny and live-provider only: it validates credentials/provider availability, not tools, sessions, workflow, REPL, or UDS behavior.

`scripts/pre-push.sh` runs fast repository and BDD quality rules plus formatting, strict workspace-shape Clippy (one feature unification, plus the standalone per-crate shapes), and architecture/contract/repository invariants. It does not run the full test, BDD, coverage, dependency-policy, or mock-E2E lanes.

Pre-push control:
- `QUECTO_PREPUSH_BASE` overrides the comparison base used to identify changed workspace packages (default `origin/master`, falling back to local `master`).

Pre-merge controls (real-LLM lane):
- `QUECTO_PROVIDER_SMOKE=1` enables `@provider-smoke` live provider checks (excluded by default)
- `OPENAI_API_KEY` supplies the OpenAI API smoke credential
- `ANTHROPIC_API_KEY` supplies the Anthropic API smoke credential
- An existing OpenAI OAuth credential in the `quecto` credential store enables the Codex smoke scenario
- `QUECTO_REAL_LLM_TIMEOUT` timeout per real-LLM shard (default `12m`)
- `QUECTO_REAL_LLM_SHARDS` shard count for real-LLM BDD (default `24`)
- `QUECTO_REAL_LLM_TAG` scenario tag to run (default `manual-real-llm`; use `real-llm-smoke` for the old smoke subset)
- `QUECTO_PREMERGE_FORCE=1` to bypass cache and rerun merge-time checks

Coverage runs in authoritative CI after `merge-requested` is applied. For manual coverage checks, use `cargo llvm-cov`.

## Directory Structure

```
~/.quecto/
  config.json              # Global configuration (a trusted ./.quecto/config.json overlays it)
  config-overlay-trust.json # Approved repo-local overlays (canonical path + sha256)
  credentials.json         # Stored API tokens (from quecto auth)
  sessions/                # Persisted conversation history (safe filename mapping)
    cli_default.json
    cli_default/            # Its session memory (spill.jsonl) and images/ (one file per SHA-256)
    repl_repl_default.json
  workspace/                # Agent working directory (files created by the agent)
```

## Documentation

Human guides (full reference). The agent `docs` tool embeds a short **operating manual** from `docs/docs-tool-embeds/` (concise deep dives) — not these full files. Its first page, `setup`, is a decision tree that routes "set up X in this repo" to one runbook page (`config`, `models`, `admission-broker`, `container-runtime` — each of the shape *Preconditions · Do · Verify · Rollback · If it fails*; the swarm row routes to the `swarm` page); the human guides below carry the same commands and expected outputs, so an agent given only the `docs` tool and a prompt like "pin the default model for this repo" can do it without a human editing files (#2024 S5).

| Guide | Description |
|---|---|
| [Getting Started](docs/getting-started.md) | Install, auth, and first run |
| [UDS Protocol](docs/uds-protocol.md) | Complete UDS command and event specification |
| [Sessions](docs/sessions.md) | Conversation persistence, context management, spill/recall |
| [Extensions](docs/extensions.md) | Add custom tools via native extensions (config-gated) or UDS extensions (external processes) |
| [Subagents](docs/subagents.md) | Spawning and controlling UDS-mode subagents with `spawn` and `agent_cmd` tools |
| [Workflow](docs/workflow.md) | UDS-only template-based workflow engine with default dormant tool availability, selector mode, guards, and live prompt injection |
| [Models & providers](docs/runtime-models-providers.md) | `models.json` registry and provider setup |
| [Container runtimes](../docs/container-runtimes.md) | Script-managed subagent environments: the `container_configs` contract and the canonical reference runtime |
| [Contributor Cookbooks](docs/contributor-cookbooks.md) | Change maps for common harness work: tools, UDS commands, providers, events, persistence, subagents, and context policy |

## Contributing

For contribution requirements, follow the root [`CONTRIBUTIONS.md`](../CONTRIBUTIONS.md). Before requesting review, run the built-in Quecto workflow through at least two complete adversarial-review loops on your change and include the workflow evidence in the PR description.

## Tech stack
Rust 2024, Tokio, reqwest+rustls, serde/serde_json, uuid, tracing, dirs, thiserror, similar, base64, sha2, rand, urlencoding, macOS unicode-normalization. Dev: cucumber 0.21, futures, tempfile, wiremock 0.6, regex.

## License

Proprietary (`LicenseRef-Proprietary`). This is a private repository; all rights reserved unless explicitly stated otherwise.
