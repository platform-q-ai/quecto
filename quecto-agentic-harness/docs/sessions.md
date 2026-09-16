# Sessions

Sessions persist conversation history so the agent remembers context across
prompts and restarts. They are the primary state mechanism for UDS agents.

## How sessions work

Each session is identified by a key. Two shapes exist today, and both are
carried unchanged by the typed `SessionIdentity` (the existing raw key, nothing
more — see [Identity](#identity-and-the-workspace-seam)):

| Kind | Key shape | Example | Created by |
|------|-----------|---------|------------|
| Named CLI session | `cli:<name>` | `cli:my-project`, `cli:default` | `quecto agent --mode uds -s my-project`; a one-shot `quecto agent -m …` without `-s` uses `default` |
| User chat | `chat-<unix-secs>-<uniq>` | `chat-1765930000-1a2b3c` | an unnamed `quecto agent --mode uds` run, and every `new_session` |

Sessions are stored as files in `<base_dir>/sessions/` by one flat layout
(`FlatSessionLayout`): `<sanitized key>.json` holds the conversation record
(a snapshot plus appended delta records with durable ordinals), `<sanitized
key>.owner` is the ownership stamp a live harness holds while it has the
session open, and `<sanitized key>/spill.jsonl` is the retained-context
namespace the `recall` tool reads. The file contains the full conversation
history (system prompt excluded — it is injected at run time and never
persisted; user messages, assistant responses, tool calls and results, the
workflow run and the historical sub-agent roster). For thinking-capable models
(Claude Sonnet 4.5+, Opus 4.5+), extended thinking blocks and their
cryptographic signatures are also persisted, enabling correct multi-turn
replay.

## Session modes

### Persistent (default)

```bash
# Unnamed UDS run: draws a fresh "chat-<unix-secs>-<uniq>" key
quecto agent --mode uds

# Uses session "cli:my-project"
quecto agent --mode uds -s my-project

# One-shot run without -s: uses session "cli:default"
quecto agent -m "hello"
```

When the agent starts, it claims and loads the session from disk (if it
exists; a key another live harness holds open is refused at startup). All
messages are appended to the session during the run. The session is saved
after each prompt completes, at every session transition, on an explicit
`persist_session`, and once more on the ordinary exit of the loop.

### Ephemeral

```bash
# No session loaded or saved
quecto agent --mode uds --no-session
quecto agent --mode uds -s -
```

The agent starts with an empty conversation. Nothing is persisted to disk, and
whatever the run spilled for in-run recall is scrubbed when the run ends.
Useful for one-off tasks or testing.

### Named sessions

Session names must contain only alphanumeric characters, hyphens, and
underscores:

- ✅ `my-project`, `feature_42`, `review2024`
- ❌ `../tmp/evil`, `my project`, `session@home`

## Session lifecycle in UDS mode

```
Agent starts
  │
  ├── Open the session: claim the key, load it from disk (unless ephemeral)
  │
  ├── Client sends prompt
  │     ├── User message added to history (and saved as a verified delta)
  │     ├── LLM response added to history
  │     ├── Tool calls/results added to history
  │     └── Session saved to disk (a clean delta, or a full record when needed)
  │
  ├── Client sends another prompt
  │     └── (same cycle, building on previous history)
  │
  ├── Client sends new_session / resume_session
  │     └── Children settled → departing session saved → (resume: target
  │         claimed and loaded) → roster reset → departing key released →
  │         the loop stands for the new identity (see the protocol reference)
  │
  ├── Last client disconnects
  │     └── Children torn down, session saved once more (ordinary exit)
  │
  └── Agent exits
        └── Socket file cleaned up
```

## Architecture (epic #1968)

The sessions capability lives under `src/application/sessions/` with the
shape the [target architecture](architecture/harness-architecture-map.md#persistence-and-session-recovery)
prescribes: use cases own every transaction and query, ports declare what the
capability needs, DTOs carry domain values only, and composition is the only
place a use case, a controller, a store or the active-session state is
constructed. The wire protocol these use cases answer is **unchanged** by the
epic — every command, field and refusal text is the same as before it
(see the [UDS Protocol Reference](uds-protocol.md), and the differential
probes recorded on each child PR of #1968).

### Use cases

Every `pub struct` under `src/application/sessions/use_cases/` is listed here;
the architecture tests refuse a use case the docs do not name.

| Use case | Catalogue | Triggered by | Owns |
|----------|-----------|--------------|------|
| `ListSessions` | #1861 | `list_sessions` | local/global discovery over saved summaries and authoritative home metadata, newest first |
| `ReadHistory` | #1856 | `get_messages`, connect-time snapshot, child transcript forwarding | stable-id cursor selection and chronological paging of the live, published or persisted transcript |
| `RecoverMessage` | #1858 | `get_message` | full-copy recovery of a possibly collapsed message (ledger, then retention store), content ranges, tool-call arguments |
| `SynchronizeTranscript` | #1857 | `sync` (idle loop and busy reader) | epoch/revision reconciliation, reset-or-delta selection |
| `ExportSessionReport` | #1859 | `get_report` | latest eligible report selection, bounded preview, the optional raw export transaction and its admission |
| `SaveSession` | #1860 | `persist_session`, post-turn and pre-turn saves, transitions, ordinary exit | the one save transaction: prompt strip/re-inject, durable ordinals, dirty latch and watermark, restore reason, roster snapshot, full vs delta |
| `ClearConversation` | #1864 | `clear_history` | clear, ledger epoch, accounting reset, retention clear, save |
| `RewindConversation` | #1865 | `rewind_to` | target resolution, truncation, retention residue removal, ledger reset, accounting reset, save |
| `StartFreshConversation` | #1862 | `new_session` | settle children → save → reset roster → clear → fresh identity → release old key → propagate → switch → reset effort/workflow → clear retention |
| `DepartingChildren` | #1862/#1863 | (collaborator of the two transitions) | fleet settlement outcome and roster replacement policy (#1937, #1938) |
| `ResumeSavedSession` | #1863 | `resume_session`, and the startup open of the loop's own session | target admission, settle → save → claim → load → roster → release → propagate → effort → history/workflow restore → switch |
| `RecallContext` | #1866 | the `recall` tool; the run-end ephemeral scrub | recall/list/clear over the retention namespace of an identity |
| `RetainContext` | #1866 | the context-pruning policy | append with receipt, the `[:{k}]` deduplication rule |
| `ListRetainedContext` | #1866 | the context-pruning policy | the retention index and presence, never content |

Each transaction has exactly one owner: the handlers in
`src/interface/cli/uds_dispatch_session.rs` admit (the streaming refusal),
request the injected use case and present the response; they sequence nothing.

### Ports

Declared only under `src/application/sessions/ports.rs` and
`src/application/sessions/ports/`, each with a contract suite under
`tests/contracts/` that runs against the real adapter:

| Port | Adapter (production) | What it supplies |
|------|----------------------|------------------|
| `SessionHomeCatalogue` | `infrastructure/persistence/session_home_catalogue.rs` (`FileSessionHomeCatalogue`) | exact authoritative home reads, first-save home recording, derived catalogue validation and recovery |
| `WorkspaceDiscovery` | `infrastructure/workspace/git_scope_discovery.rs` (`GitScopeDiscovery`), using `filesystem_scope.rs` | canonical execution directory and real Git common-dir/worktree grouping; observable discovery failure |
| `SessionStore` | `infrastructure/persistence/session_store.rs` (`FileSessionStore`) | claim/release/load/save/save_delta/save_clean_delta/exists/list, keyed by `SessionIdentity` |
| `ContextSpillStore` | `infrastructure/persistence/context_spill.rs` (`FileContextSpillStore`) | append/recall/list_entries/has_entries/clear/scrub_sync of the retention namespace |
| `SessionExportPort` | `infrastructure/session_export.rs` (`FileSessionExport`) | the raw export writer (records, manifest, checksum) |
| `DurablePrefixObservation` | `application/durable_prefix.rs` (`DurablePrefixLatch`) | the agent loop's dirty-prefix latch the save drains |
| `WorkflowRunSource` | `infrastructure/persistence/session_snapshot_sources.rs` | the workflow run the save records |
| `HistoricalRosterSource` | `infrastructure/persistence/session_snapshot_sources.rs` | the sub-agent rows the save records as history |
| `TurnAccountingReset` | `interface/cli/uds_turn_accounting.rs`, `uds_session_switch_runtime.rs` | usage/context/pending reset when history is replaced |
| `FreshSessionIdentityGenerator` | `infrastructure/persistence/fresh_session_identity.rs` | wall clock + pid + counter → a fresh `chat-…` identity |
| `FleetSettlement` | `composition/fleet_settlement.rs` (adapts `TerminateAllDelegatedAgents`) | settle delegated children before a switch (#1938) |
| `DelegatedChildrenRoster` | `infrastructure/tools/delegated_roster.rs` | live delegated rows and roster replacement |
| `SessionKeyPropagation` | `interface/cli/uds_session_switch_runtime.rs` | the agent loop and its session-aware tools adopt the new identity |
| `SessionSwitchRuntime` | `interface/cli/uds_session_switch_runtime.rs` | effort reset, workflow reset/restore |

### Home authority and recovery (#2009)

`FileSessionStore` stores versioned optional home authority alongside each global
transcript as a `.home` sidecar. Ordinary full and delta transcript saves preserve
existing authority bytes, including unsupported or corrupt metadata. Only a new
persistent identity can receive its initial home; existing legacy records are not
automatically associated. Ephemeral runs write neither transcripts nor homes.

The derived `home.catalogue` is discardable. Listing validates it against
authoritative records, reports missing/corrupt/stale data and rebuilds by atomic
replacement. A failed replacement returns valid discovered rows with diagnostics,
not a transcript rewrite. Orphan home files without committed transcripts are not
rows. Exact-key admission reads authority independently of catalogue health.

`SessionHomeContext` is an application observation collaborator, not another
query/save/restore owner. `ListSessions`, `SaveSession` and `ResumeSavedSession`
retain those responsibilities. Resume admission rechecks canonical facts after
ownership admission for both explicit resume and startup; a discovery group is
not permission to execute history in another directory.

### DTOs and the active session

`src/application/sessions/dto/` carries requests, results and errors as domain
values (messages, ids, identities, ledger positions); no `serde_json::Value`,
no `AgentEvent`, no transport type. Every refusal text a client can see is a
DTO `Display` impl. The one live-conversation read model is the
application-owned `ActiveSessionState` (`active_session.rs`: typed identity,
the `ConversationLedger` published transcript with its bounded full-copy
ledger and sync frontier, the retention backstop, and the persistence state —
watermark, sticky durable-prefix latch, killing exit). It is created once per
loop by composition; the interface publishes into it and reads through the
use cases. The `sessionKey` every presenter reports (`get_state`,
`get_session_stats`) is read from this identity — no interface tracker holds
a copy (#1979).

### Composition

- `src/composition/sessions.rs` — `build_session_handles` (the `FileSessionStore`
  over the one `FlatSessionLayout`, `ListSessions` and its controller, the
  fresh-identity generator) and `build_retention_handles` (the
  `FileContextSpillStore` over the same layout, and the recall/retain/list graph).
- `src/composition/active_session.rs` — the per-loop graph over the
  `ActiveSessionState`: history, recovery, sync, save, clear, rewind, fresh,
  resume and their controllers.
- `src/composition/session_report.rs` — the export root
  (`<base>/artifacts/session-exports`), `FileSessionExport`, `ExportSessionReport`.
- `src/composition/retention.rs` — `RecallContext`, `RetainContext`,
  `ListRetainedContext` over one store.
- `src/composition/fleet_settlement.rs` — the fleet adaptation of the switch.

The interface holds plain handle structs (`src/interface/cli/uds_session_handles.rs`,
`retention_handles.rs`) filled through `CliComposition`; it names neither the
composition layer nor an adapter, converts no raw key, and reaches no store
method — the architecture tests under `tests/architecture/sessions_*.rs` pin
every one of these rules with exact, decrease-only inventories.

### Retained context: who owns what

Sessions is the single owner of durable retained context — the
`ContextSpillStore` port, the identity-keyed recall/list/clear selection, the
id append and deduplication. The context-pruning policy
(`src/application/context*.rs`, the agent loop) decides *when and what* to
retain and consumes the narrow `RetainContext`/`ListRetainedContext` handles,
never a store method; it also allocates the spill ids (`turn{n}:{tool}:{idx}`,
`turn{n}:msg:{role}`). The `recall` tool is an adapter over `RecallContext`:
schema parse, result formatting, diagnostics.

One raw conversion remains by design: the tools capability's
`Tool::set_session_key(String)` port is raw, so `RecallTool` converts the key
it is handed with `SessionIdentity::from_persisted_key` — the one admitted
infrastructure conversion site, pinned exactly by the architecture tests.
Typing that port is a tools-capability change and future work outside this
epic. The tool registry's startup key (`Session::build_key("cli", name)` in
`agent_tool_registry.rs`) is the same capability's raw input and is admitted
on the same terms.

### Identity and the workspace seam

`SessionIdentity` (`src/domain/session_identity.rs`) is the **existing raw key
only**: `ephemeral()`, `named_cli(name)`, `user_chat(key)`, `fresh_chat(secs,
uniq)`, and the total persistence round-trip `from_persisted_key`. It has no
scope field, no variant, no path conversion and no workspace behaviour, and
this epic adds none.

Folder-aware discovery (#2009, parent #2001) keeps `SessionIdentity` opaque
and retains `FlatSessionLayout` and the global transcript store. Home metadata
is separate from identity: repository/worktree grouping controls discovery,
not the permission to restore history in a different execution directory.
Canonical exact-folder identity applies outside Git; the nearest repository
wins inside Git. Legacy records without home metadata remain unassociated.
Invalid or unsupported home metadata is unavailable, never legacy-unscoped.

The first discovery slice excludes metadata search and executable cross-folder
actions. Open-original, fork, locate and first association belong to later
children of #2001; discovery must never silently substitute history reuse for
an unavailable action.

## Context management

As conversations grow, the agent manages context automatically:

### Context window

The agent tracks estimated token usage against an application-level context
budget. When the conversation exceeds `max_context_tokens` (configurable,
default `200000`), the agent applies context pruning:

1. **Spilling at creation**: Tool outputs *and* conversation (user/assistant)
   messages are written to the session's retention namespace when they are
   created, so anything later collapsed or dropped can still be recovered with
   the `recall` tool
2. **Tool output collapsing**: Once the session accumulates more than
   `context_collapse_after_tool_calls` tool calls, the oldest tool outputs are
   replaced with compact recall stubs. The trigger counts tool calls
   cumulatively across prompts within a session. Current config default: `50`.
   Set it to `4294967295` (`u32::MAX`) to disable collapse.
3. **Conversation message collapsing**: An independent dial,
   `context_collapse_after_messages`, keeps the most recent N conversation
   messages in full and replaces older ones with one-line recall stubs.
   Exempt: the system prompt, spill manifest, in-flight user prompt, and the
   `pin_recent_turns` most recent turns. Defaults to 50 (mirroring the
   tool-call collapse default); set to `4294967295` (`u32::MAX`) to disable.
4. **Demotion ladder**: When the conversation still exceeds the effective
   budget, messages are demoted down a ladder — full content is collapsed to
   recall stubs first (oldest first), and only if the budget is still
   exceeded are stubs removed entirely (their content stays on disk). Pinned
   and tail-pinned (`pin_recent_turns`, default `2`) content is never
   demoted; if the pinned set alone exceeds the budget, a
   `context_prune` warning is logged and the `ContextPruned` audit event
   records `budget_unmet`.

The effective budget is the smaller of `max_context_tokens` and the active
model's context window when the model registry declares one.

### Spill and recall

Retained content is appended to `<base_dir>/sessions/<sanitized key>/spill.jsonl`
(the session's retention namespace; `new_session`, `resume_session`,
`clear_history` and `rewind_to` move or clear it with the conversation). Once
the namespace is non-empty, the conversation carries a pinned, static guidance
message that points the model to `recall("list")`; it deliberately contains no
spill count, IDs, or previews, so the provider-visible prompt prefix remains
byte-identical as the spill set grows. `recall("list")` returns the complete
live index on demand, and the `recall` tool description advertises that route.

When a message is collapsed, the compact stub looks like:

```
[bash: ls -la (2450 tokens) — recall("turn5:bash:0")]
```

The agent can call `recall("turn5:bash:0")` to retrieve the original content
from the retention namespace, even after the original message has been
collapsed or dropped by the sliding window.

## Inspecting sessions

### Via UDS commands

```json
{"type": "get_state"}
```

Returns the slim supervision projection: the session key, generation,
streaming state, model, effort and workflow identity.

```json
{"type": "get_messages"}
```

Returns the newest bounded page of conversation history (#1061).

```json
{"type": "get_messages", "count": 5}
```

Returns the last 5 messages (omit `count` for the newest bounded history page; the response's `before`/`hasMoreBefore` fields page older history, #1061).

```json
{"type": "get_session_stats"}
```

Returns the session key, token usage, message counts, and cost estimates.

### Clearing history

```json
{"type": "clear_history"}
```

Clears all messages except the system prompt. Drains any pending follow-up
or steer messages. Fails if the agent is currently streaming. See
UDS Protocol Reference (`docs {"name":"uds-protocol"}`) for full details.

## Configuration

Session behavior is configured in `config.json` under `agents.defaults`:

```json
{
  "agents": {
    "defaults": {
      "max_context_tokens": 100000,
      "context_collapse_after_tool_calls": 3
    }
  }
}
```

| Field | Default | Description |
|-------|---------|-------------|
| `max_context_tokens` | `200000` | Application-level token budget before context pruning (clamped down to the model's declared context window when known) |
| `context_collapse_after_tool_calls` | `50` | Collapse the oldest tool outputs once the session exceeds N tool calls. Set to `4294967295` (`u32::MAX`) to disable |
| `context_collapse_after_messages` | `50` | Collapse the oldest conversation (user/assistant) messages to recall stubs once the session exceeds N live messages. Set to `4294967295` (`u32::MAX`) to disable |
| `pin_recent_turns` | `2` | How many most-recent turns the context ceiling never demotes or drops |

## See also

- UDS Protocol Reference (`docs {"name":"uds-protocol"}`) — `list_sessions`, `resume_session`, `new_session`, `persist_session`, `get_state`, `get_messages`, `get_message`, `sync`, `get_report`, `get_session_stats`, `clear_history`, `rewind_to`
- Harness architecture map (`docs/architecture/harness-architecture-map.md`) — the sessions capability in the layer model
- Subagents (`docs {"name":"subagents"}`) — each subagent gets its own session
