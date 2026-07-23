# Clean Architecture gap map for `quecto-tui`

Parent epic: [#1149](https://github.com/platform-q-ai/quecto/issues/1149). First slice: [#1150](https://github.com/platform-q-ai/quecto/issues/1150).

## Constraints and migration guardrails

- No big-bang rewrite and no intentional user-visible behavior changes.
- Each extraction slice must be independently shippable and keep existing TUI BDD/headless harness coverage green.
- Keep protocol DTOs and runtime I/O at infrastructure boundaries; application/domain code should receive stable TUI vocabulary instead.
- Temporary architecture-test allowlists are acceptable only when they are narrow, explained, and point back to #1149 or a child issue.
- Prefer early vertical slices for ledger-sync/feeds and session/resume/master history over resurrecting deleted child-backfill machinery.
- Do not treat #1196 transcript memory caps as Clean Architecture work; that is a separate product follow-up.

## Current layer shape

`src/domain/` is currently a placeholder for pure TUI vocabulary and has no exported value objects or ports yet.

`src/application/` is thin. Its concrete logic is mostly around `session_payloads`, so orchestration for sessions, subagents, feeds, workflow, workspace, and git remains elsewhere.

`src/infrastructure/` contains real UDS and terminal/process implementations, but `infrastructure::client::{Client, Command, Event, SubagentInfoEvent, SubagentWorkflow, ...}` are also used directly as the shared language inside `interface/`.

`src/interface/` owns most policy and orchestration. `interface::App` directly holds runtime adapters, component state, protocol channels, subagent/feed state, recovery maps, history cursors, workflow toggles, git state, timers, and view/session state. For example, `App` imports `Client`, `Command`, `Event`, and `SubagentWorkflow` from infrastructure at `src/interface/app.rs:6`, owns the concrete `Client` at `src/interface/app.rs:47`, and stores feed event channels as `(String, Event)` at `src/interface/app.rs:178-180`.

## Dependency and DTO gaps

The main production `interface` modules that currently import or name infrastructure client DTOs include:

- `src/interface/app.rs:6` imports `Client`, `Command`, `Event`, and `SubagentWorkflow`; `App` stores the concrete client and protocol event channels.
- `src/interface/feed_state.rs:1` stores a `mpsc::Sender<Command>` in feed state, coupling feed policy to UDS commands.
- `src/interface/app_subagent_feed.rs:32-50` opens direct child UDS feeds, sends `GetState`/`Sync`, and fans raw `Event` values into UI-owned state.
- `src/interface/app_ledger_sync.rs:11-16` and `src/interface/app_ledger_sync.rs:52-57` decide when to send protocol `Sync` commands.
- `src/interface/app_subagents.rs` accepts `SubagentInfoEvent` vectors as bar/roster input.
- `src/interface/app_subagent_state.rs` stores `SubagentInfoEvent` inside tracked UI state.
- `src/interface/app_events.rs` routes raw UDS events and extracts protocol result text.
- `src/interface/app_response.rs:297-305` deserializes `SubagentInfoEvent` directly from a response payload.
- `src/interface/tui_harness.rs` and `src/interface/tui_harness_events.rs` intentionally use raw DTOs to simulate protocol traffic.

Raw JSON parsing is also still present in production interface paths:

- `src/interface/app_ledger_sync.rs:32-37` parses a JSON response into `interface::ledger_sync::SyncDelta`.
- `src/interface/ledger_sync.rs:8-17` models sync deltas as JSON-backed DTOs, stores messages as `serde_json::Value`, and converts them to chat entries through interface recovery code at `src/interface/ledger_sync.rs:20-46`.
- `src/interface/ledger_sync.rs:49-57` probes sync capability from raw JSON.
- `src/interface/app_models.rs` parses model-list JSON for selector state.
- `src/interface/app_methods.rs` handles session stats, resume selector data, and message replacement from raw JSON payloads.
- `src/interface/range_accumulator.rs` applies range updates from raw JSON values.

## Known exceptions for now

- Harness and BDD support modules may continue to use protocol DTOs and JSON fixtures while tests simulate UDS traffic (`tui_harness*`, `*_tests.rs`). Stricter production rules in #1151 should allow this explicitly rather than forcing test rewrites.
- Render adapters and terminal/process infrastructure can stay concrete infrastructure; the Clean Architecture target is to move policy decisions and DTO mapping, not to hide every side effect behind premature abstractions.
- Component/view modules may continue to own rendering state and widgets. The gap is when they parse protocol payloads or decide use-case transitions.

## Post-#1194 extraction surface

The #1194 follow-up series correctly landed feed and ledger-sync behavior in `interface/`, but that expanded the surface to extract rather than completing Clean Architecture work.

Current policy-heavy modules include:

- `app_subagent_feed`: connect-on-discover feed opening, initial `GetState`/`Sync`, command fan-out, and feed insertion.
- `feed_state`: feed authority, cursor, pending revision, sync capability, and transcript state.
- `app_ledger_sync`: ledger-advanced handling, sync request dedupe/cursor policy, delta application, caught-up/pagination decisions, and authority promotion.
- `ledger_sync`: upsert-by-id transcript semantics and resync replacement.
- `app_subagent_stream` and `app_subagent_panel`: subagent stream/panel transitions, focus-as-view-switch behavior, workflow stickiness, and result extraction.
- `app_response` master attach backfill path, including `get_subagents` response parsing.
- `app_paged_history`: master paged history request correlation, retry policy, and stub auto-recall routing.

Important non-gap: subagent focus-time reconcile and parent-crumb transcript warming for children were removed by the #1194 series. Future CA slices must preserve source-scoped roster authority and must not resurrect the deleted child multi-source reconcile/backfill design. Master attach backfill and paged history remain valid extraction targets.

## First low-risk vertical slices

1. **#1151 — stricter architecture tests with temporary allowlists.** Capture current production exceptions for infra DTO imports and raw JSON parsing, with comments pointing to #1149/#115x.
2. **#1152/#1153/#1154 — vocabulary, commands/events, and boundary mappers.** Introduce small domain value objects and application command/event types before moving larger use cases.
3. **#1155 — session/resume/master history and ledger-sync use cases.** Extract request correlation, retry, resume, master history, and sync policy behind application services.
4. **#1156 — subagent lifecycle, roster, and feed-manager transitions.** Preserve source-scoped roster authority while moving policy out of `App`.
5. **#1157 — workspace files and git branch ports.** This is comparatively independent and can proceed in parallel.
6. **#1158/#1159/#1160 — multi-feed gateway and interface DTO/JSON reduction.** Replace raw `infrastructure::client` language in interface paths with application/domain language.
7. **#1161/#1162 — thin `interface::App` and enforce DTO isolation/use-case ownership.** Tighten tests once temporary allowlists are burned down.

## Tracking allowlists

#1151 should create the first explicit allowlists for production interface imports/parsing that would fail stricter rules today. Initial candidates are the modules listed in **Dependency and DTO gaps**, with separate allowances for test/harness files. Each entry should include a removal issue number from #1153 through #1162 so new violations cannot hide inside broad legacy exceptions.
