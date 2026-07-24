# Clean Architecture target model for `quecto-tui`

Parent epic: [#1149](https://github.com/platform-q-ai/quecto/issues/1149).

This note describes the desired Clean Architecture end state for `quecto-tui` and, more importantly, how to get there safely. The goal is not to move code into `domain/` and `application/` for its own sake. The goal is to move real TUI policy and orchestration out of `interface::App` behind typed, behavior-preserving boundaries.

## Executive recommendation

For this TUI, the gold-standard end state is best described as a **functional core with an imperative shell**, not a ceremonial four-layer architecture.

`quecto-tui` is a protocol-client UI. Most of its hard logic is not business-domain modeling; it is correlation, projection, authority, lifecycle, and command-ordering policy. The right end state is therefore:

```text
infrastructure/
  wire I/O, DTO deserialization, DTO-to-typed-input mapping
  the only place raw UDS protocol JSON should be interpreted

application/
  one module per policy area
  pure state transitions: typed input + prior state -> new state + ordered effects
  no terminal, widgets, Tokio handles, concrete Client, or raw protocol DTOs

domain/
  deliberately thin shared vocabulary
  IDs, cursors, future-safe enums, small invariant-bearing value objects
  thin is acceptable; do not invent a rich domain model just to fill the folder

interface/
  App as composition root, input adapter, effect executor, runtime owner, and view controller
  widgets, layout, render projection, overlay state, terminal mechanics, and async task supervision
```

Success is measured by this question:

> Can every correlation, authority, generation, pagination, and lifecycle rule be tested as a pure transition with no Tokio runtime, no terminal, no `Client`, no `ChatEntry`, and no `serde_json::Value`?

Today the answer is mostly no. This refactor is complete when the answer is yes for the major policy areas.

## Current architectural diagnosis

The crate already has `domain/`, `application/`, `infrastructure/`, and `interface/`, but the effective architecture is still UI-centered:

- `domain/` is currently a placeholder.
- `application/` is thin; the main foothold is `session_payloads.rs`, which is an interim typed-payload extraction and still accepts `serde_json::Value`.
- `interface::App` and its `app_*.rs` satellites own protocol correlation, command ordering, transcript recovery, feed authority, roster authority, run state, widget state, and runtime handles.
- `infrastructure::client::{Command, Event, ...}` currently acts as the shared language across layers.
- `FeedState` mixes pure feed-sync policy (`epoch`, `rev`, `authority`, `pending_rev`, transcript) with runtime handles (`mpsc::Sender`, `JoinHandle`).
- `handle_response` is a high-value seam: a string-matched dispatcher over raw response payloads where many correlation rules currently live.

The codebase is nevertheless in good shape for this migration:

- cohesive substructures already exist (`SessionView`, `SubagentUi`, `RewindFlow`, `ModelRegistry`, `PendingHistoryPage`, `StreamRenderCoalescer`, `LedgerTranscript`),
- subtle invariants are documented inline with issue references,
- and there is a strong parity suite: unit tests, headless harness tests, and BDD features.

## Target direction

`interface::App` should become primarily:

- a composition root,
- an input/protocol adapter,
- an ordered effect executor,
- a runtime owner for terminal, async tasks, channels, and feed handles,
- a holder of transient widget/render state,
- and the renderer/view controller.

It should no longer own the meaning of history cursors, request correlation, ledger authority, roster-source authority, feed epochs, transcript recovery batches, or run-state generations.

A representative target flow is:

```text
UDS bytes
  -> infrastructure wire DTO
  -> infrastructure feature mapper
  -> typed application input
  -> bounded application service
  -> domain/application state transition
  -> ordered application effects
  -> interface/runtime effect executor
  -> infrastructure gateway/feed/workspace adapter
  -> interface view projection and render
```

UI input enters through the interface controller directly into typed application use cases. Only protocol input goes through infrastructure DTO mapping.

## Dependency rules

The target dependency rules are:

- `domain` must not depend on `application`, `interface`, `infrastructure`, terminal/widget types, Tokio channels/tasks, filesystem/process types, or `serde_json`.
- `application` must not depend on `interface`, concrete infrastructure clients, terminal/widget types, Tokio task handles, or raw UDS DTOs.
- Infrastructure adapters deserialize and validate wire/protocol DTOs, then translate them into typed application inputs.
- Application services consume typed, protocol-independent values and return ordered effects.
- Interface code projects typed state into widgets, display strings, colors, layout, notifications, and rendered chat entries.

Moving raw `serde_json::Value` parsing from `interface` to `application` is only an interim extraction, not the final target. The final target is typed mapping at infrastructure/adapter boundaries and pure application transitions over protocol-independent values.

## Application service shape

Prefer per-policy-area functional cores:

```rust
fn apply(state: &mut AreaState, input: AreaInput, now: Instant) -> Vec<AreaEffect>
```

Guidelines:

- Use **per-area** input/effect enums. A single global application event enum risks mirroring UDS and recreating the god object as an enum.
- Effects must be ordered.
- Migrated policy must not call `send_command` inline.
- The interface/runtime executor sends returned effects through the existing FIFO command path in vector order.
- Send failures must re-enter the relevant application service as typed inputs so pending state can roll back.
- Long-running runtime work should generally be modeled as effects that spawn runtime tasks, with results re-entering as typed inputs, rather than by making application services async.
- Avoid async-trait proliferation and one trait per wire command.

## Migration principle: vertical slices, not horizontal phases

Do **not** implement this epic as separate horizontal phases such as:

1. create all domain value objects,
2. define all application commands/events,
3. map all DTOs,
4. then move use cases.

That risks speculative wrappers, duplicated state, and a late high-risk cutover.

Instead, migrate behavior-complete vertical slices. Each slice should introduce only the domain types, application inputs/effects, ports, and mappers required by that slice. Existing behavior must remain stable and covered by characterization tests.

For each slice, use this pattern:

1. Identify current observable behavior and existing tests.
2. Add missing characterization tests before moving policy.
3. Introduce typed wire mapping at the boundary.
4. Move one bounded policy/state machine into `application`/`domain`.
5. Return ordered effects for the interface/runtime to execute.
6. Preserve existing UI rendering and command ordering.
7. Delete the old policy path after parity is proven.

Avoid long-lived production code that writes both old and new state. During an interim migration, each PR must name the single source of truth for every field it touches.

## Mechanical pre-slices

Before feature migration, do two small enabling slices.

### A. Split pure feed-sync state from feed runtime handles

Today `FeedState` mixes runtime ownership and policy. Split it into roughly:

- `FeedRuntime`: command sender, task handle, connection supervision details,
- `FeedSyncState`: epoch, revision, authority, pending revision, capability state, typed transcript state.

This should be a behavior-preserving struct split. It unblocks later ledger/feed extraction and prevents Tokio handles from being dragged inward.

### B. Establish the typed response/event mapper convention

Create an `infrastructure` mapper convention for converting wire DTOs/raw payloads into typed application inputs. `handle_response` may keep its broad match temporarily, but migrated arms should immediately map raw payloads to typed values before policy runs.

This is intentionally shallow at first. The point is not to migrate every command at once; it is to prevent every vertical slice from inventing its own mapping style.

## Bounded policy areas

These are the meaningful policy areas in the TUI. They are not all “domain” in the pure sense; many are application projection/orchestration state.

### Workspace context projection

Current behavior:

- list workspace files for `@files`,
- prefer hardened `git ls-files`,
- fall back to bounded filesystem walk,
- sanitize unsafe paths,
- refresh Git branch for the footer without blocking input/render.

Likely inward concepts:

- `WorkspaceFile`,
- `GitBranch`,
- `WorkspaceFilesPort` or `WorkspaceContextPort`,
- `GitHeadPort` if kept separate.

Keep fuzzy matching, popup behavior, footer rendering, and background task execution in the interface/runtime.

### Master history, pagination, and rewind

Master history is deliberately different from child ledger sync.

Current behavior:

- attach-time `get_messages`,
- exact request correlation,
- page-before cursor lifecycle,
- retries for failed/lost requests,
- partial/full backfill guards,
- resume/rewind/new/clear invalidation,
- no duplicates or interior gaps.

Likely inward concepts:

- `MessageId`,
- `HistoryCursor`,
- `CorrelationId`,
- `HistoryPage`,
- `PendingHistoryPage`,
- `MasterHistoryState`.

Do not create a generic “history for any agent” abstraction that erases the master/child distinction.

### Transcript assembly, stub recall, and turn recovery

This policy deserves its own boundary because it is shared by resume/history/recovery/ledger paths.

Current behavior:

- common fully streamed path does zero fetches,
- missing or partial content triggers fetch-by-ref,
- tool-call cardinality affects recovery,
- open tool calls force recovery,
- request ID and message ID must match,
- chunks are reassembled before application,
- recovered batches replace the original range atomically,
- spawn tool cards may be display-suppressed but still count for ref completeness.

Likely inward concepts:

- `TranscriptMessage`,
- `ToolInvocation`,
- `ToolResult`,
- `ToolCallId`,
- `TurnRefSet`,
- `ContentRange`,
- `RecoveryBatch`,
- `RecoveryTarget`.

Domain/application types must not depend on `ChatEntry`. Projection to `ChatEntry` belongs at the interface boundary.

### Child feed and ledger synchronization

Child feeds use warm direct feeds and ledger sync. This must remain distinct from master attach history.

Current behavior:

- new feed starts warm, not authoritative,
- sync capability alone does not grant authority,
- feed becomes authoritative only after applying a sync delta,
- epoch/revision rules protect against stale deltas,
- resync clears stale transcript,
- `ledger_advanced` may record pending revision until capability is known,
- synced authority suppresses duplicate legacy live transcript mutation,
- focus may request catch-up without reopening.

Likely inward concepts:

- `FeedId`,
- `FeedEpoch`,
- `LedgerRevision`,
- `FeedAuthority`,
- `SyncCapability`,
- `FeedSyncState`,
- typed `SyncDelta`,
- typed ledger transcript.

Application/domain state must not contain `mpsc::Sender` or `JoinHandle`.

### Subagent roster, lifecycle, focus, retention, and feed discovery

Current behavior:

- source-scoped roster authority,
- recursive descendant updates,
- prevention of reparent/hijack by the wrong source,
- optimistic spawn entries,
- exited-agent grace and GC,
- retained child sessions and warm-feed caps,
- active target fallback,
- stale queued event rejection,
- workflow snapshot recording.

Likely inward concepts:

- `SubagentId`,
- `SubagentStatus`, with unknown/future-safe values,
- `SubagentSnapshot`,
- `RosterSource`,
- `SubagentRoster`,
- `SubagentLifecycle`,
- `AgentTarget`.

Separate raw transport identity from sanitized display labels where possible. If current behavior uses sanitized IDs as map keys, characterize that before changing it.

Panel cursor, highlight, glyphs, elapsed text formatting, and viewport behavior remain interface concerns.

### Inference settings: model catalog, model selection, effort

Current behavior:

- reload model catalog when selector opens,
- parse model list JSON,
- route model changes to master or active subagent,
- update master optimistically,
- avoid optimistic child updates until authoritative state resync,
- avoid late master responses clobbering focused child state.

Likely inward concepts:

- `ModelId`,
- `ProviderId`,
- `ModelOption`,
- `ModelCatalog`,
- `AgentTarget`,
- narrow gateway requests for `ListModels`, `SetModel`, and `GetState`.

Keep selector rows, fuzzy behavior, display markers, and sanitization projection in the interface.

### Workflow projection and automation

The TUI currently projects workflow state; it does not own the workflow engine.

Current behavior:

- hide dormant selector snapshots,
- distinguish transient empty state from real completion,
- preserve sticky child workflow progress,
- seed selected child workflow from roster snapshots,
- toggle workflow automation settings,
- cap synthesized snapshot steps.

Likely inward concepts:

- `WorkflowMode`, including unknown/future-safe values,
- `WorkflowStep`,
- `WorkflowProgress`,
- `WorkflowIssue`,
- `WorkflowAutomation`,
- `WorkflowSnapshot`,
- `WorkflowProjectionState`.

Keep rendering widths, glyphs, colors, boxes, truncation, and panel layout in the interface.

Do not add TUI use cases for check/skip/uncheck/bind/guard evaluation unless the TUI actually gains those capabilities.

### Session metadata, stats, and lifecycle

Current behavior:

- list resume candidates,
- resume selected session,
- start new session,
- clear history,
- update footer stats quietly or show chat-visible stats,
- preserve command ordering and state resyncs around lifecycle changes.

Likely inward concepts:

- `SessionId`,
- `ResumeCandidate`,
- `ResumeTarget`,
- `SessionStats`,
- ordered lifecycle effects.

Infrastructure should map session/stats payloads into typed values. Interface should keep selector row formatting and user-facing status wording.

The current `application::session_payloads` module is an interim foothold, not a final precedent. It should be retired or converted to typed mapper/application boundaries as this slice lands.

### Conversation run control

Move this late, after target/focus, transcript, and feed boundaries are stable.

Current behavior:

- abort-aware run generations,
- stale `AgentEnd` suppression after abort,
- submit vs follow-up routing,
- Ctrl+C clear-before-abort priority,
- master/child run state transitions,
- tool counters for recovery,
- deferred subagent completion notes.

Likely inward concepts:

- `AgentRunState`,
- `SubmitIntent`,
- `RunEvent`,
- `TurnState`,
- ordered effects for prompt/follow-up/abort/get-message requests.

## Recommended migration sequence

This sequence differs from a pure low-risk order. It is ordered by stability, leverage, and code temperature.

### 0. Guardrails and characterization readiness

Before implementation slices, add/update guardrails:

- no new raw protocol parsing in interface feature handlers,
- no `serde_json` in new domain modules,
- no `interface` imports from `domain`/`application`,
- no concrete `Client`, `Command`, or `Event` imports in migrated application modules,
- no inline `send_command` from migrated policy,
- allowlists only when narrow and issue-linked,
- ratchet current `serde_json` and infrastructure DTO usage in production interface files so counts can decrease but not increase without an allowlist.

### 1. Mechanical pre-slices

- Split `FeedState` into runtime and pure sync state.
- Establish the infrastructure mapper convention around `handle_response`/event handling.

### 2. Workspace files and Git branch projection

Use this as the first full vertical implementation slice because it is low coupling and exercises all layers without UDS transcript risk.

Preserve file caps, Git-first/fallback behavior, path sanitization, `.git/HEAD` size cap, worktree support, nonblocking background loading, in-flight dedupe, and branch polling interval.

### 3. Master attach history, pagination, and rewind

Promote this earlier than the original plan. It is the best real proof that the pattern handles correlation policy: exact request IDs, broadcast response rejection, retry, lifecycle invalidation, and no interior gaps.

Preserve exact request matching, rejection of foreign/broadcast pages, lost-response retry, no duplicate/interior-gap history, partial/full backfill semantics, lifecycle invalidation, and rewind replacement behavior.

### 4. Typed transcript assembly, stub recall, and turn recovery

Unify typed transcript reconstruction before moving ledger sync.

Preserve zero-fetch common path, cardinality/open-tool recovery triggers, duplicate/mismatched ref handling, chunked content assembly, atomic range replacement, spawn suppression without losing recovery cardinality, and child/master recovery parity where current behavior requires it.

### 5. Child ledger synchronization and feed authority

Move this after typed transcripts exist and after current feed/ledger churn has quieted.

Preserve warm-sync initial state, authority only after sync delta, epoch/revision rules, resync replacement, pending revision behavior, caught-up behavior, catch-up-on-focus behavior, and no resurrection of deleted child backfill/reconcile logic.

### 6. Models, effort, session metadata, and workflow projection

These are lower-risk and can proceed in parallel with the transcript/feed work if different contributors are involved.

For models/effort, preserve reload-on-open, cached fallback, provider inference, no optimistic child update, authoritative child `get_state` resync, and late master response isolation.

For workflow, preserve dormant selector hiding, selected `0/N` visibility, transient empty-state handling, explicit completion semantics, synthesized-step cap, automation flag mirroring, and forwarded child workflow isolation.

For session metadata, preserve resume filtering/fallbacks, user-facing wording, quiet footer stats vs visible stats, resume/new/clear command ordering, recovery/history invalidation, and current workflow retention behavior.

### 7. Subagent roster, lifecycle, focus, retention, and feed discovery

Move roster/focus after feed/workflow/session types exist.

Preserve source-scoped authority, optimistic spawn grace, exited retention/GC rules, active target fallback, retained-session/feed caps, stale-event rejection, read-only marker semantics, and panel navigation/rendering behavior.

### 8. Unified master/child turn control and gateway completion

Move run-state and command-target orchestration last.

Preserve Ctrl+C and Escape semantics, submit vs follow-up routing, stale end suppression, command FIFO ordering, send-failure feedback and rollback, deferred completion-note behavior, and no rendered behavior changes.

## Checkpoints

After the workspace slice and again after master history, explicitly review whether:

- the per-area effect model is sufficient,
- the effect executor preserves FIFO ordering,
- send failures re-enter application state correctly,
- mapper conventions are consistent,
- application tests are replacing `App` harness tests for pure policy without deleting parity coverage.

If not, revise the pattern before transcript, ledger, roster, or run-control work.

## Testing strategy

Each slice should keep three layers of tests:

1. **Infrastructure mapper tests**: raw wire fixture to typed value/event.
2. **Application transition tests**: typed input plus prior state to new state plus ordered effects.
3. **Existing interface/BDD characterization tests**: same visible behavior and emitted command semantics.

Useful existing coverage includes:

- workspace/files: `workspace_files_tests.rs`, `app_git_tests.rs`, `tui_file_mention.feature`,
- model: `app_models_tests.rs`, `model_selector_tests.rs`, `app_model_focus_1085_tests.rs`,
- workflow: `workflow_bar_tests.rs`, `app_subagent_workflow_sticky_tests.rs`, subagent layout/parity features,
- session/history: `app_attach_backfill_tests.rs`, `app_paged_history_tests.rs`, `app_paged_history_review_tests.rs`, `app_rewind_response_tests.rs`, `tui_paged_history.feature`,
- recovery: `app_events_1060_*`, `tui_end_of_turn_refs.feature`, range accumulator tests,
- ledger: `ledger_sync_tests.rs`, `app_ledger_sync_tests.rs`, roster authority tests,
- subagents: `app_subagents_tests.rs`, `app_subagent_*`, subagent parity/layout/read-only features,
- run control: Ctrl+C, Ctrl+D, Escape abort, streaming stability, note coalescing tests.

New inward tests should be added. Existing harness/BDD tests should not be removed simply because a policy moved inward. Rendered-output changes in structural slices should be treated as regressions unless deliberately approved as product changes.

## Ports and effects guidance

Introduce ports only when a real dependency inversion exists.

Likely justified ports/effects:

- `WorkspaceFilesPort` / `WorkspaceContextPort`,
- `GitHeadPort` or a method on workspace context,
- narrow gateway effects/adapter for typed agent requests, extended per slice,
- feed-open/feed-close/sync-request effects,
- fakeable `now` and correlation-id inputs for history/retry tests.

Prefer pure application services where practical. Avoid one trait per wire command and avoid async-trait proliferation unless the use case genuinely requires it.

## Anti-patterns to avoid

- Creating a broad domain vocabulary before use cases need it.
- Building one giant `ApplicationState` god object.
- Defining an application event enum that mirrors UDS DTOs.
- Defining one global effect enum that becomes an indirect copy of every UDS command.
- Moving `serde_json::Value` parsing from interface to application and declaring victory.
- Letting domain/application depend on `ChatEntry`, `Footer`, `ModelEntry`, `WorkflowBarState`, theme, ANSI, terminal, Tokio task handles, or `mpsc` channels.
- Unifying master history and child ledger sync prematurely.
- Relaxing exact request correlation.
- Changing user-visible rendering during structural slices.
- Collapsing raw identity and sanitized display text without characterization.
- Moving interface/runtime concerns inward just to shrink files.
- Chasing a line-count target for `App` instead of removing policy decisions.

Interface/runtime concerns that should generally stay outside include:

- Tokio `select!`,
- terminal raw mode and resize,
- Kitty protocol,
- stream render coalescing,
- fuzzy search,
- overlays and panel cursor state,
- chat viewport/layout,
- spinner animation,
- colors/glyphs/widths,
- mouse selection and clipboard mechanics,
- task/channel ownership for live feeds.

## Suggested issue restructuring

Treat the current child issues as cross-cutting concerns and slice deliverables, not strict horizontal phases:

| Existing issue | Recommended role |
|---|---|
| #1150 — Gap doc + guardrails | Superseded in part by this document; remaining scope is sequence step 0: characterization readiness and guardrail definition. |
| #1151 — Stricter arch tests + allowlists | Implement sequence step 0 mechanically: import-direction tests plus ratcheting counts for interface raw JSON/DTO usage, with narrow issue-linked allowlists. |
| #1152 — Domain value objects | Cross-slice checklist: introduce only needed IDs/cursors/value objects. Domain may remain thin. |
| #1153 — Application commands/events | Reframe as per-area typed inputs/effects, not one global command/event vocabulary. |
| #1154 — Map UDS DTOs | Establish mapper convention, then implement incrementally per migrated flow. |
| #1155 — Session/resume/master history/ledger-sync | Split conceptually into session metadata, master history, transcript recovery, and child ledger sync; promote master history earlier. |
| #1156 — Subagent lifecycle/roster/feed manager | Start with FeedState runtime/policy split; extract roster/focus later after feed/workflow/session boundaries exist. |
| #1157 — Workspace files + git branch ports | Use as the first full vertical implementation slice. |
| #1158 — AgentGateway application port | Do not build a broad port up front; grow narrow gateway effects/adapters from real slices. |
| #1159/#1160 — Reduce infra DTO imports / raw JSON in interface | Convert to ratcheting guardrails plus burn-down through slices. |
| #1161/#1162 — Thin App / enforce tests | Finalize after migrated policies have single inward owners; success is policy removal, not line count. |

## Definition of done for #1149

The epic is done when:

- meaningful domain/application types exist because migrated policies require them,
- domain remains deliberately thin and free of ceremony,
- infrastructure maps protocol DTOs into typed application inputs for migrated areas,
- application owns migrated correlation, pagination, recovery, sync, roster, lifecycle, and run-state policy,
- migrated application services are testable as pure transitions with ordered effects,
- interface owns rendering, input adaptation, runtime/task ownership, effect execution, and transient widget state,
- `App` no longer interprets raw protocol JSON for migrated areas,
- migrated policy no longer sends commands inline,
- existing TUI behavior remains stable under BDD/headless harness coverage,
- architecture tests prevent regression toward UI-centered policy,
- no generic inner command/effect layer merely duplicates UDS.
