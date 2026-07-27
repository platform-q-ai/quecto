# PRD: TUI context-window footer + real chat sessions

Status: ready to implement · Owner: SQ · Target crate(s): `quecto` (core/UDS), `quecto-tui`

Two related TUI problems, bundled because both touch session/usage plumbing.

---

## A. Context-window footer shows a meaningless number

### Problem
The TUI footer's context number shows ~`176` for a one-line "Hi" turn. That value is a
local chars≈/4 heuristic over the conversation text only — it excludes the system
prompt overhead and **all tool-schema tokens**, so it does not represent context-window
occupancy and is meaningless to the user. The provider's real prompt-token count for the
same turn is ~`2246` (visible in `/session` as `↑`).

### Root cause (confirmed)
- The real value is already computed: `agent_usage.rs` → `context_input_tokens = usage.prompt_tokens`
  (the provider's prompt tokens for the latest request = true current context size). It is
  also already carried on `AgentResult.input_tokens`.
- But the displayed field is wired to the estimate:
  - `../quecto-agentic-harness/src/application/agent_loop.rs` -> `finalize_text_response` (~L448) sets
    `context_tokens = context_pruning::estimate_total_tokens(messages)`.
  - same in the streaming finalize path (~L679).
- `../quecto-agentic-harness/src/shell/cli/uds.rs` and `uds_multi.rs` call the session's context-token setter with
  `estimate_total_tokens(...)` — these are **init-time seeds** (before any turn). After a turn,
  `record_agent_result` overwrites the session's context size from `AgentResult.context_tokens`.
- The UDS `get_state` emits `contextTokens`; the TUI footer reads it
  (`src/shell/app_methods.rs`, `app_events.rs`). A test
  (`session_stats_footer_uses_context_tokens_not_cumulative_input`) already pins that the footer
  must use `contextTokens`, NOT cumulative input — so the right number is the latest turn's real
  prompt tokens, not the running sum.

### Requirements
1. `AgentResult.context_tokens` must equal the provider's real prompt tokens for the latest turn:
   `if usage.context_input_tokens > 0 { usage.context_input_tokens as usize } else { estimate_total_tokens(messages) }`.
   Apply in BOTH `finalize_text_response` and the streaming finalize path in `agent_loop.rs`.
2. Drop the redundant `set_context_tokens(estimate_total_tokens(...))` seeds in `uds.rs`/`uds_multi.rs`
   (or have them seed `0` / the estimate only until the first real turn). The post-turn value must
   come from `AgentResult.context_tokens` (now the real value).
3. Keep `estimate_total_tokens` for internal pre-send pruning decisions only — do not display it.
4. Leave `/session` cumulative `↑input ↓output` as-is, but it is a **cost** metric (cumulative),
   not window occupancy — label it as cumulative if convenient.

### Acceptance
- After "Hi" + reply, the footer context number ≈ the provider's prompt tokens (~2k with the tool
  set loaded), not ~176, and matches the latest turn's `↑` order of magnitude.
- Multi-turn: the number reflects the **current** context size (latest prompt tokens), not a sum.

### Tests
- Unit: `finalize_text_response` / streaming finalize set `context_tokens` from
  `context_input_tokens` when present; fall back to the estimate when `context_input_tokens == 0`.
- Keep `session_stats_footer_uses_context_tokens_not_cumulative_input` green.

---

## B. Real, distinct, tidy chat sessions

### Problem
Every interactive TUI/REPL conversation is written to one shared key
(`repl_repl_default` / `cli_default`), so there are no distinct chats. `/resume` lists **all**
session files in `~/.quecto/sessions/` — including internal sub-agent/agent-manager/command
sessions (`cli_subagent`, `cli_agent_manager_*`, `cli_quecto_command_agent-*`) — labeled with raw
keys, no titles or timestamps. Result: the user can't find old chats; the list is junk.

### Confirmed design decisions
1. **New chat each launch** + `/new` to start another + `/resume` picker of past chats.
2. **Hide internal sessions** from `/resume` via a separate namespace.
3. **Each session entry shows a date + time** alongside its name (absolute, e.g. `2026-06-20 18:42`),
   in addition to the title and message count.

### Requirements

**B1. User-chat namespace**
- Add a shared constant `USER_CHAT_PREFIX = "chat-"`.
- Interactive launch with no explicit `--session`: start a NEW chat with a unique key
  `chat-<unix_seconds>` (disambiguate collisions within the same second, e.g. append a short
  counter/random suffix). Where the interactive key is currently defaulted to "default":
  `../quecto-agentic-harness/src/shell/repl/mod.rs` (the `session_name.unwrap_or("default")` path).
- Sub-agent / agent-manager / command sessions keep their existing keys (NOT `chat-`-prefixed),
  so they are excluded from the user-facing list. The legacy `default` session is also excluded
  (treated as legacy; not migrated).

**B2. `/new` command** (REPL + TUI)
- Starts a fresh `chat-*` session (new key, empty history). Prior chats remain on disk.

**B3. `/resume` picker**
- Lists ONLY sessions whose key starts with `chat-`.
- Each entry shows: **title · date+time · message count**, e.g.
  `Fix the auth bug…           2026-06-20 18:42   (12 msgs)`
  - **title**: first user message text, trimmed to ~50 chars; `(untitled)` if none yet.
  - **date+time**: absolute, from the session file's last-modified time (last activity).
    Format `YYYY-MM-DD HH:MM` (local). (Optional: also show a relative hint like `· 3m ago`.)
  - **message count**: number of user+assistant messages (exclude system/memory entries).
- Sort newest-first (by last-modified).
- Selecting an entry switches the active session key to that chat and loads its messages.

**B4. Server-side `list_sessions` (UDS)**
- Return, per user chat, metadata: `{ key, title, updated_at, message_count }` filtered to the
  `chat-` namespace. (`../quecto-agentic-harness/src/shell/cli/uds*.rs` + `../quecto-agentic-harness/src/infrastructure/persistence/session_store.rs`.)
- `session_store` computes: `title` = first user message; `message_count` = count of non-system
  messages; `updated_at` = file mtime (epoch or RFC3339).
- Prefer filtering to the user-chat namespace server-side; the TUI just renders.

**B5. Footer / `/session` label**
- Show the chat **title** (or key) plus its date+time, so the active chat is identifiable.

**B6. Do not break ESC‑ESC rewind** (jump to an earlier turn within the *current* chat) — separate
feature; it should now operate on a clean single-conversation session.

### Edge cases / migration
- Legacy `default` session: left on disk, excluded from `/resume` (not `chat-`). No migration.
- Internal sessions: unchanged on disk; excluded from `/resume` by namespace.
- Empty new chat (no messages yet): excluded from `/resume` (or shown as `(untitled)` — choose;
  excluding until first message is cleaner).
- Two launches same second: ensure unique keys.
- `--session <name>` explicit: honored as today (bypasses auto-new); such a key is shown in
  `/resume` only if it is `chat-`-prefixed (document this, or always include explicit names — pick
  one and note it).

### Acceptance
- Launching the TUI starts an empty new chat; sending a message titles it from that message.
- `/new` starts another empty chat without losing the previous one.
- `/resume` shows only real chats, newest-first, each as **title · date+time · count**; selecting
  one loads it.
- No sub-agent/agent-manager/command/`default` sessions appear in `/resume`.

### Tests (hermetic — temp dirs / injected `CliContext.base_dir`; no `~/.quecto`, no real network)
- New launch generates a `chat-*` key; `/new` generates a distinct one.
- `list_sessions` returns only `chat-*` with correct title/count/updated_at; excludes
  `cli_subagent`/`cli_agent_manager_*`/`cli_quecto_command_agent-*`/`default`.
- Title derivation (first user message, truncation, untitled).
- `/resume` selection loads the chosen session's messages.
- Date+time present and formatted per entry.
- Follow existing TUI harness conventions (`src/shell/tui_harness.rs`) and the
  `*_tests.rs`/`#[path]` sibling-file pattern (750-line cap).

---

## Gates (must pass before merge)
- `cargo build --workspace`; `cargo test --lib -p quecto -p quecto-tui`;
  `cargo test -p quecto --test architecture --test contracts --test repo_docs --test workflow_docs`.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo fmt --check`.
- Region coverage ≥ **87%** for both `quecto` and `quecto-tui`
  (`QUECTO_COV_THRESHOLD` is 87; pre-push enforces it).
- Pre-push also runs machete, deny, and the real-LLM suite (~140s; `QUECTO_SKIP_REAL_LLM=1` to skip).
- If any BDD feature files describe the old single-`default` or `/resume` behavior, update them.
- Use only current model ids (e.g. `claude-haiku-4-5`, `gpt-5.2`) in any test config.

## Suggested file touch-list
- `../quecto-agentic-harness/src/application/agent_loop.rs` (A: finalize + streaming finalize)
- `../quecto-agentic-harness/src/shell/cli/uds.rs`, `uds_multi.rs` (A: drop estimate seeds; B: list_sessions metadata)
- `../quecto-agentic-harness/src/infrastructure/persistence/session_store.rs` (B: list with title/mtime/count, namespace filter)
- `../quecto-agentic-harness/src/shell/repl/mod.rs` (B: per-launch `chat-` key, `/new`, `/resume`)
- `src/shell/app_methods.rs` / `app_response.rs` / resume-selector component
  (B: `/new`, `/resume` picker rendering with title · date+time · count; footer label)
- Shared constant for `USER_CHAT_PREFIX`.
