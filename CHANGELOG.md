# Changelog

## 0.25.0 (2026-04-04)

### Added
- **`agent_cmd await` command** (#612): Block until a sub-agent reaches a terminal state (idle, exited, timeout, or error). Returns a structured JSON result with status, reason, elapsed time, and optional workflow snapshot. Supports `timeout` (default 300s) and `idle_timeout` (default 5s) parameters. The `idle_timeout` window correctly filters brief idle gaps during auto-continue workflow steps. Only one `await` per agent is allowed — a second returns `"another_await_active"` immediately.
- **Exit signal channel for subagents** (#612): The reaper task now signals waiting `await` calls with the child's exit code or signal via `tokio::sync::watch` channels. This enables `await` to report accurate exit reasons (e.g. `exit_code_1`, `signal_9`) instead of a generic result.
- **`SubagentAwait` audit event** (#612): When `await` returns, a `subagent_await` event is emitted on the parent's audit log with `agent_id`, `status`, `reason`, and `elapsed_ms`.

### Changed
- **`agent_cmd` tool definition updated**: Command enum and description now include `await`. Schema includes `timeout` and `idle_timeout` properties.
- **`SubagentEntry` extended**: Added `exit_signal_tx` field for reaper-to-await signaling.

## 0.24.0 (2026-03-27)

### Added
- **Spawn forwards --config, --workflow, --workflow-guards** (#611): Child agents spawned with the `spawn` tool can now inherit workflow configuration from the parent via `config`, `workflow`, and `workflow_guards` parameters.
- **Append-only audit log** (#609): Durable event recording for all tool calls, LLM turns, workflow steps, and subagent operations. JSON-lines format with envelope (timestamp, session, turn).

### Fixed
- **TUI workflow shortcuts** (#608): Removed broken Ctrl+Shift+A/N toggle shortcuts that conflicted with terminal emulator bindings.

## 0.23.0 (2026-03-20)

### Added
- **`agent_cmd` expanded to 13 commands** (#547): New commands: `follow_up`, `get_messages`, `set_model`, `clear_history`, `get_subagents`, `get_extensions`, `reload_extensions`. `set_model` supports both `model` and `provider`+`model_id` forms with empty-string rejection and partial-parameter errors.
- **`agent_cmd kill` command** (#559): Terminate a specific subagent by ID — sends SIGTERM, aborts monitor task, removes from registry. Handled locally (not via UDS).
- **Server-side workflow auto-nudge** (#562): Wires existing `auto_continue_nudge()` and `completion_nudge()` domain methods into the UDS dispatch loop. Agents are now self-driving — they auto-continue through workflow steps and prompt to pick the next issue on completion. No TUI or external nudger required. Enables meta-agents managing hundreds of self-driving subagents.
- **Workflow event emitter support** (#562): `register_workflow_tool()` now accepts an optional `WorkflowEventEmitter` callback and returns the shared `WorkflowStateHandle`. `workflow_state` events can be broadcast to all UDS clients.
- **TUI: Sci-fi workflow header bar** (#563): Persistent header bar showing workflow progress with phase-aware true-colour backgrounds (RED/GREEN/CI/REVIEW), box-drawing separators, block-element progress bar, and angle-bracket phase tags. Alacritty-safe (no Nerd Font required). Hidden when no workflow issue is active.
- **TUI: `Ctrl+Shift+A` / `Ctrl+Shift+N` keybindings** (#563): Toggle auto-continue and completion nudge notifications. New `CtrlShift(char)` key variant in Kitty protocol parser.
- **TUI: Mouse selection visual highlight** (#546): Selected text shows reverse-video during click-and-drag. `apply_line_highlight()` handles ANSI-escaped text correctly. Highlight stored separately from extraction buffer to prevent escape leakage.
- **TUI: Clipboard notification** (#546): "Copied N chars to clipboard" toast on successful OSC 52 copy.
- **TUI: Auto-remove exited subagent bars** (#540): Exited subagent status bars disappear after a 5-second grace period. `TrackedSubagent` wrapper records `exited_at` timestamp. GC runs on each spinner tick.

### Fixed
- **`agent_cmd` UDS response filtering** (#555): `send_uds_command` now parses JSON to find `"type":"response"` events, skipping broadcast noise (tokens, agent_start, etc.) that arrives first in multi-client mode.
- **`agent_cmd` write-half shutdown** (#557): Removed `writer.shutdown()` which caused the server's reader loop to exit and abort the broadcast writer before the response was delivered. Connections now stay open until the response is read.
- **`agent_cmd` query responses visible** (#538): `agent_cmd` query results (get_state, get_messages_tail, get_session_stats) are now shown in the TUI chat. Mutations (prompt/steer/abort) remain suppressed. Extracted `suppress_tool_box(tool_name, args)` and `is_subagent_tool()` functions.
- **Table column truncation** (#550, #555): Replaced proportional column scaling (`.max(3)`) with iterative shrink algorithm that freezes narrow columns at natural width. Tool names like `spawn`/`agent_cmd` no longer truncated to 3-4 chars.
- **Inline code in table cells** (#550): `Event::Code`, `Event::SoftBreak`, and `Event::HardBreak` now check `in_table` and append to `current_cell` instead of `current_line`.
- **Ctrl+C clears editor first** (#536): Ctrl+C now checks editor content before deciding — clears text if non-empty, only aborts if editor is already empty. Extracted `ctrl_c_action()` pure function.

### Changed
- **License**: Changed from MIT to proprietary (`LicenseRef-Proprietary`).
- **Socket utilities extracted**: `reap_stale_sockets`, `SocketGuard`, `bind_secure_socket` moved from `uds.rs` to `uds_socket.rs`.
- **`agent_cmd` tests extracted**: Tests moved to `agent_cmd_tests.rs` for 750-line limit compliance.
- **quecto-tui bumped to 0.2.0**: Reflects significant feature additions (workflow bar, mouse highlight, subagent bar expiry, table fixes).

## 0.22.0 (2026-03-16)

### Fixed
- **Full Anthropic provider parity with Pi and OpenCode** (#437): Comprehensive gap analysis identified and fixed 16 differences causing 500 errors. See "Added" and "Changed" sections below for details.
- **Auto-enable adaptive thinking for Opus 4.6 / Sonnet 4.6** (#432): Quecto was sending `output_config.effort` without `thinking` and including `temperature` alongside effort for 4.6 models, causing 500 errors from the Anthropic API. The provider now always emits `thinking: {type: "adaptive"}` for 4.6 models even when `thinking_level` is `None`, and suppresses temperature. Matches pi-mono's behavior.
- **User-agent header for OAuth** (#432): Changed from `quecto/0.12.0 (external, cli)` to `claude-cli/2.1.75` to match pi-mono's Claude Code identity headers.
- **BDD: spill-store steps panic with no Tokio reactor** (#426): Converted two `async fn` BDD step definitions to synchronous functions with inline `tokio::runtime::Runtime`, matching the pattern used by all other async BDD steps.
- **BDD: web_fetch tests blocked by SSRF on localhost** (#425): Changed `#[cfg(test)]` to `#[cfg(any(test, feature = "test-support"))]` for the SSRF bypass logic and replaced the blanket `allow_restricted_hosts` boolean with a per-host allowlist, so only the wiremock mock server's host:port is exempted while SSRF protection scenarios continue to pass.
- **BDD: duplicate step definition causing cucumber ambiguity** (#428): Removed duplicate `the tool result should not contain` step.
- **BDD: beta headers mock returns 404** (#429): Fixed by #432 — BDD scenario updated to verify the `fine-grained-tool-streaming` beta header is absent (now GA).

### Added
- **Claude Code system prompt for OAuth tokens** (#437-1): OAuth requests now prepend "You are Claude Code, Anthropic's official CLI for Claude." to the system prompt, matching pi-mono's behavior. Required by the `claude-code-20250219` beta.
- **`interleaved-thinking-2025-05-14` beta header** (#437-2): Both API key and OAuth auth now send this header for non-4.6 models (omitted for 4.6 where interleaved thinking is built-in).
- **`fine-grained-tool-streaming-2025-05-14` beta header restored** (#437-3): Both Pi and OpenCode still send this despite "GA" status. Restored for parity.
- **Tool name remapping for OAuth** (#437-4): New `claude_code.rs` module maps tool names to Claude Code canonical casing (`read` → `Read`, `bash` → `Bash`, etc.) on outbound requests and reverse-maps on API responses. Uses zero-allocation `eq_ignore_ascii_case`.
- **Thinking block replay in multi-turn conversations** (#437-5): New `ThinkingBlock` domain type (`Normal` with thinking text + cryptographic signature, `Redacted` with opaque data). Stored on `Message`, propagated through `LlmResponse`, persisted in `FileSessionStore`, and replayed in assistant messages with signatures for API correctness.
- **`signature_delta` SSE event handling** (#437-6): The SSE accumulator now captures thinking block signatures from `signature_delta` events, enabling complete thinking block capture for multi-turn replay.
- **`Accept: application/json` header** (#437-10): All Anthropic requests now include this header, matching the `@anthropic-ai/sdk` defaults that Pi uses.
- **`agent_cmd` tool** (#421): Send commands to spawned UDS subagents — `steer` (interrupt + redirect), `follow_up` (queue message), `abort` (cancel run), `get_state` (check status). Enables orchestration of multiple concurrent agents.
- **`spawn` now launches UDS-mode agents** (#421): Subagents are spawned as `quecto agent --mode uds` processes with Unix domain sockets, replacing the previous stdin-based approach. Enables async, multi-turn interaction with child agents via `agent_cmd`.

### Changed
- **Anthropic provider: unified header construction** (#437): `apply_auth_headers()` replaced by `apply_headers()` which combines auth, beta, version, and identity headers in one place. `build_beta_header()` is now a standalone function that conditionally includes betas based on model and auth type.
- **Anthropic provider: `fine-grained-tool-streaming` beta header restored** (#437-3): Reversed the removal from #432 — both Pi and OpenCode still send it, so we do too.
- **`sanitize_surrogates` is zero-allocation** (#437-14): Returns `Cow::Borrowed` instead of cloning the string. Rust strings cannot contain surrogates, so this is a defence-in-depth no-op.
- **`web_fetch` SSRF bypass uses per-host allowlist** (#425): The test-support SSRF bypass is now a `Vec<String>` of allowed host:port pairs instead of a blanket boolean, providing more precise test isolation.

### Documentation
- Rewrote `docs/subagents.md` for UDS-mode spawn and `agent_cmd` tool (#423).
- Added `--effort` flag to UDS protocol startup flags reference (#419).

## 0.21.0 (2026-03-15)

### Fixed
- **Default `effort=low` for Claude 4.6 models** (#416): Sonnet 4.6 and Opus 4.6 default to `effort: high` on the Anthropic API when `output_config` is omitted, causing excessive token usage and 529 overloaded errors under load. The Anthropic provider now emits `output_config: {effort: "low"}` for these models when no explicit effort is set, matching Sonnet 4.5 behaviour per the migration guide.
- **`model_context_window_exceeded` stop reason** (#416): Claude 4.5+ models may return `model_context_window_exceeded` when the context window is exhausted. This is now parsed as `StopReason::MaxTokens` so context handling (spill/collapse) fires correctly.

### Added
- **`--effort` CLI flag** (#416): Override effort level for 4.6 models (`--effort low|medium|high|max`). Takes precedence over config and env var.
- **`agents.defaults.effort` config field** (#416): Set default effort level in `config.json` (`"low"`, `"medium"`, `"high"`, or `"max"`).
- **`QUECTO_AGENTS_DEFAULTS_EFFORT` env var** (#416): Override effort level via environment variable. Validated at config load time.
- **`EffortLevel::parse()`** (#416): Domain-layer parsing for effort level strings, shared across CLI, config, and REPL paths.

## 0.20.0 (2026-03-13)

### Added
- **`clear_history` UDS command** (#408): Reset conversation history in-place without restarting the agent. Preserves system prompt, drains pending queue, errors if agent is streaming.
- **`--disable-tool` flag** (#402): Remove specific tools from the agent's registry before the session starts. Repeatable (`--disable-tool bash --disable-tool spawn`). Works in both one-shot and UDS modes. Permanently blocks disabled tool names from UDS re-registration.
- **`web_fetch` tool** (#364): Fetch a URL and return content as readable text. Strips HTML by default; `raw: true` for JSON/markdown. Config: `tools.web.fetch.enabled`.
- **SSRF protection**: `web_fetch` rejects requests to loopback, link-local, private RFC-1918, and cloud metadata addresses.
- **Multi-tool native extensions**: `NativeExtension::with_tools()` constructor for extensions with multiple tools.
- **Comprehensive SpawnTool BDD scenarios** (#401): 31 BDD scenarios (100 steps) covering argument parsing, agent ID validation, workspace restriction inheritance, network passthrough, constructors, tool definition, stub-mode execution, debug trait, and timeout constant.

### Changed
- Web tools (`web_search`, `web_fetch`) consolidated into single `"web"` extension (was `"web_search"`). Each tool independently config-gated.

### Fixed
- **Workflow guard ignores command patterns inside quoted strings** (#405): Guard no longer falsely blocks commands that mention guarded patterns in string arguments.
- **`inject_system_prompt` skips injection when manifest at `messages[0]`** (#407): System prompt injection correctly handles manifest messages.

### Performance
- **Line-by-line scan with early exit in `recall()`** (#373): Avoids reading entire spill file for single-entry lookups.
- **In-memory index cache for spill store** (#375): Eliminates repeated disk reads for spill index.
- **Clone-on-write in `normalize_messages`** (#374): Avoids cloning messages that don't need normalization.
- **Zero-copy forwarding in `RefreshableProvider`** (#372): Eliminates unnecessary request cloning.
- **Batch micro-optimizations** (#377, #383, #385, #386, #387, #389, #376, #379): FNV hashing, pre-allocated buffers, reduced allocations across hot paths.
- **Replace `chrono` with `std::time`** (#395): Lighter timestamp operations, removes heavy dependency.
- **Replace `FallbackProvider` with zero-copy `ProviderRouter`** (#394): Eliminates Arc overhead and dynamic dispatch on the fast path.
- **Remove `serde_yaml`** (#392): Hand-rolled YAML frontmatter parser for skill files, removes heavy dependency.
- **Remove `image` crate** (#391): Send images as-is without client-side resize, removes heavy dependency.
- **Release profile optimizations** (#390): LTO, strip, `panic=abort`, `codegen-units=1` for smaller, faster binaries.

## 0.19.0 (2026-03-09)

### Extension system migration (Phases 1–4)

The extension system has been completely redesigned. The old subprocess-based `ScriptTool` model (TOML manifests, shell/Python/Node scripts, filesystem hot-reload) has been replaced with two complementary mechanisms:

**Native extensions (#351):**
- Compiled-in Rust tools registered conditionally from `config.json`
- `web_search` is the first native extension (Brave Search API + DuckDuckGo fallback)
- Zero overhead when disabled; child agents inherit the same config
- Config keys: `tools.web.brave.enabled`, `tools.web.brave.api_key`, `tools.web.duckduckgo.enabled`

**UDS extension protocol (#352):**
- External processes connect to the agent's Unix socket and register tools via JSON-lines
- New commands: `register_tools`, `unregister_tools`, `tool_result`
- New event: `execute_tool` (routed to the registering client, not broadcast)
- Automatic lifecycle: disconnect = auto-unregister + `extensions_changed` broadcast
- Shadow protection: core tool names cannot be overridden
- 30-second timeout for tool execution with graceful error handling

**ScriptTool removal (#353):**
- Deleted `ScriptTool`, `ExtensionManifest`, `ScriptExtension`, `ExtensionWatcher`
- Removed filesystem-based extension discovery, TOML manifest parser, subprocess execution, process group management, hot-reload watcher
- Simplified `ExtensionRegistry` to register/all_tools/snippets only
- Removed `is_script()` from `Extension` trait
- Removed `hot_reload_interval` from UDS loop args
- Net: −2,200 lines of code removed

**Documentation rewrite (#354):**
- `docs/extensions.md` fully rewritten for native + UDS extension model
- `docs/uds-protocol.md` updated with `register_tools`, `unregister_tools`, `execute_tool`, `tool_result`, disconnect cleanup, sequence diagrams
- `README.md` architecture section updated
- `reload_extensions` UDS command documented as deprecated no-op

### Breaking changes

- Script extensions (`extension.toml` + executable scripts) are no longer supported
- The `<workspace>/extensions/` directory is no longer scanned
- `reload_extensions` UDS command is now a no-op (returns success immediately)
- `Extension::is_script()` trait method has been removed

## 0.18.0 (2026-06-10)

### Added
- **`--persist` flag for UDS mode**: `quecto agent --mode uds --persist` keeps the agent alive when all clients disconnect, rather than exiting. Useful for long-lived background agents.
- **Complete extension system**: Extensions are now fully wired end-to-end — `swap_registry` for atomic hot-swap, live UDS commands (`get_extensions`, `reload_extensions`), fingerprint-based hot-reload watcher (mtime + size polling), `extensions_changed` broadcast event to all connected clients.
- **Workflow tool and `WorkflowGuard`**: Built-in `workflow` tool for BDD/TDD step tracking (status, check, uncheck, reset, skip, set_issue, clear_issue). `WorkflowGuard` blocks `git commit`/`git push` when configured steps are incomplete. Steps are fully configurable in `config.json` — no hardcoded defaults. `guard_commit` and `enforce_commit_after_step` config options.
- **Real-time tool streaming over UDS**: `tool_execution_start` / `tool_execution_end` events carry `toolCallId` for per-call correlation. Incremental `token` events forwarded in real time from `chat_stream_incremental()`.
- **Configurable workflow guards**: Any command can be blocked before any step via `config.json`. `guard_commit` controls `WorkflowGuard` registration.
- **`--config` flag**: Override the config file path at startup (`quecto --config /path/to/config.json`).
- **`--persist` UDS flag**: Agent stays alive after all clients disconnect (opt-in).
- **Steer/abort via `CancelSlot`**: Real interrupt-after-tool semantics for `steer` and `abort` UDS commands via a race-free `Idle → Armed → Fired` state machine.
- **Multi-client UDS event bus**: `tokio::sync::broadcast` delivers events to all connected clients simultaneously. Up to 64 concurrent clients. RAII `ClientGuard` for accurate client-count tracking. Lagged clients receive a re-sync notification.
- **Incremental token streaming**: `chat_stream_incremental()` delivers real-time `StreamEvent` tokens through a channel, forwarded as `token` events over UDS.
- **`--network` flag**: Per-invocation network access for the `bash` tool (disables nsjail network namespace).
- **Mid-session OAuth refresh**: `RefreshableProvider` intercepts 401 errors, refreshes the token, rebuilds the provider, and retries automatically — no session interruption.
- **`set_model` UDS command**: Switch model at runtime; takes effect on the next prompt.
- **`get_session_stats` UDS command**: Token usage and cost statistics for the current session.
- **`get_messages_tail` UDS command**: Fetch the last N messages without loading the full history.
- **`spawn` tool**: Headless CLI agent can spawn background subagents for long-running tasks.
- **`recall` tool**: Retrieve previously spilled (collapsed) tool outputs by spill ID.
- **`web_search` tool**: Brave Search and DuckDuckGo integration.

### Changed
- **UDS replaces stdin/stdout transport**: The persistent agent mode now exclusively uses Unix Domain Sockets. The old stdin/stdout RPC transport has been removed.
- **`AgentMode` enum removed**: Simplified to a `uds_mode: bool` flag internally.
- **Gateway, Telegram, cron, heartbeat, voice, and message bus removed**: Dead subsystems stripped to reduce surface area and complexity.
- **Tool registry caches definitions at registration**: Sorted once, reused for all subsequent lookups — no repeated allocations.
- **`StopReason::parse` renamed**: Cleaner API surface.
- **`Message` constructors simplified**: `::system`, `::user`, `::assistant`, `::tool` — reduced boilerplate throughout.
- **Shared SSE pump extracted**: OpenAI and Anthropic providers share a unified SSE streaming implementation.
- **`filter_orphan_tool_pairs` optimised**: Faster orphan detection with diagnostic output (`OrphanDiag`).
- **Stale socket reaping**: Sockets older than 24h are removed on startup.
- **Socket permissions**: Created with `chmod 0600` (owner-only).

### Fixed
- `PromptOutcome::Error` no longer kills the UDS event loop — the agent stays alive and accepts new commands after an LLM error.
- Streaming unroutable model errors now surface as `agent_error` response events rather than crashing the loop.
- `tool_use` input is always a JSON object, never `null` — prevents provider serialization errors.
- Poisoned mutex handling in extension registry reload.
- Tool registry sync on extension reload.
- nsjail `/etc/ca-certificates`, `/etc/resolv.conf`, and `/etc/hosts` bind-mounts for SSL and DNS inside the jail.
- Consistent `expires_at` safety margin across all OAuth paths.
- `OAuthTokenResponse.refresh_token` is now optional per RFC 6749.
- Denylist hardened against bash encoding/escaping bypass variants.
- UDS socket permissions and `XDG_RUNTIME_DIR` preference.

### Source PRs
#269–#349 (UDS transport, multi-client bus, extension system, workflow tool, streaming, security hardening, OAuth refresh, --persist, --config, --network)

---

## 0.15.0 (2026-03-03)

### Added / Updated
- **RPC mode for CLI agents**: `quecto agent --mode rpc` now exposes a JSON-lines protocol for long-lived headless operation and external tooling.
- **Cancel support**: `CancelFlag` is plumbed through request paths to support aborting in-flight calls when possible.
- **User message content blocks**: user messages now support structured content (including inline image blocks) with provider capability filtering.
- **Cross-provider message normalization**: conversation messages are normalized between providers for consistent tool-call handling.
- **Anthropic streaming improvements**: true incremental SSE streaming is implemented with richer stream event handling.
- **Anthropic provider enhancements**:
  - Per-call cost tracking and pricing metadata.
  - Extended-thinking mode support.
  - Tool batching, `tool_choice`, SSE usage accounting, and stop-reason reporting.
- **Extensions and operational tooling updates**:
  - Added agent-manager dashboard and heartbeat `.pi` extension.
  - Dashboard + heartbeat integration updates.
  - Pi extension refactor work across several components (`Cow<str>`, `ImageBlock`, spawn-blocking grep path, etc.).

### Documentation
- Updated user-facing docs to describe the new `--mode rpc` headless mode and abort-friendly RPC behavior.
- Reviewed recent API/behavior changes from the last 10 PRs and aligned docs/versioning for release notes.

### Source PRs reviewed
- #233, #182, #188, #184, #181, #229, #185, #175, #216, #228

## 0.14.0

See `0.14.0` baseline release that aligned docs and architecture references.
