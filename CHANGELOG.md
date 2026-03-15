# Changelog

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
