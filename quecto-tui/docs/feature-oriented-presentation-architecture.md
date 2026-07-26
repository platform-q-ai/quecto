# Feature-oriented presentation architecture for `quecto-tui`

Parent epic: [#1149](https://github.com/platform-q-ai/quecto/issues/1149).

`quecto-tui` is a presentation adapter for `quecto agent --mode uds`. It does not own a separate business domain. Its stable boundaries should follow user-facing harness capabilities rather than reproducing a ceremonial `domain/application/infrastructure/interface` stack inside the TUI.

## Target capabilities

Production ownership should converge toward these feature-oriented modules:

- `shell`: composition root, top-level routing, event loop, terminal/runtime ownership.
- `protocol`: UDS client, wire DTOs, raw framing/deserialization, and DTO-to-feature mapping.
- `conversation`: conversation controller/state, history pagination, transcript recovery, and view projection.
- `sessions`: resume/new/clear/stats presentation flows.
- `agents`: subagent roster, lifecycle, feeds, ledger sync, focus, retention, and view projection.
- `workflow`: workflow projection and controls.
- `inference`: model and effort presentation.
- `workspace`: files, Git context, and autocomplete coordination.
- `components`: reusable widgets and rendering primitives.

These are presentation modules aligned with harness capabilities. They must remain coupled to the public UDS protocol, not to private `quecto-agentic-harness` internals.

## Interim compatibility map

The current crate still uses `application/`, `domain/`, `infrastructure/`, and `interface/` directories because existing architecture checks and callers depend on them. During migration, treat those directories as compatibility buckets, not as the final target model:

| Current location | Interim role | Target direction |
|---|---|---|
| `interface/app*.rs` | shell plus feature controllers still co-located in `App` | split by `shell`, `conversation`, `sessions`, `agents`, `workflow`, `inference`, and `workspace` |
| `interface/components/*` | widgets and render helpers | `components` |
| `infrastructure/client*.rs` | UDS client and wire protocol boundary | `protocol` |
| `infrastructure/process.rs`, `terminal.rs`, `signals.rs`, `render.rs` | runtime/terminal adapters | `shell` runtime adapters or explicit infrastructure seams under `shell` |
| `infrastructure/workspace_files.rs` | workspace filesystem/Git adapter | `workspace` boundary adapter |
| `application/model_payloads.rs` | typed mapper for `list_models` wire payloads (#1220) | `protocol` mapping feeding `models` |
| `application/session_payloads.rs` | typed parsing foothold for session payloads | `protocol` mapping feeding `sessions` |
| `domain/` | conversation history/recovery invariants (#1221) | only keep values that encode real invariants |

Each migration PR should preserve one source of truth for every moved state cluster and avoid long-lived dual writes.

## Boundary rules

- Raw UDS framing, raw JSON interpretation, and deserialization belong in `protocol`/transport mapping code.
- Feature controllers and views receive typed protocol results where that removes ad hoc wire-shape interpretation.
- Do not introduce a second global command/event hierarchy that mirrors UDS.
- Do not require per-feature `Input`/`Effect` enums, gateway traits, or ports unless a real dependency inversion or state-machine boundary needs them.
- Pure policy modules must not depend on terminal/widget types, Tokio task handles/channels, concrete clients, filesystem/process APIs, or raw JSON.
- Rendering, fuzzy matching, overlays, layout, panel cursors, terminal mechanics, task supervision, and channel ownership remain presentation/runtime concerns.
- Shared IDs/newtypes are introduced only when they encode an invariant or prevent a demonstrated class of mistakes.
- Preserve command FIFO semantics and existing observable behavior.

## Policy worth extracting

Extract pure state machines only where the client owns non-trivial rules, such as:

- master-history request correlation, pagination, retry, and invalidation;
- transcript reconstruction, stub recall, and turn recovery;
- child feed epoch/revision/authority transitions;
- source-scoped subagent roster authority, lifecycle, focus, and retention;
- run generations and stale-event suppression.

Straightforward flows such as listing sessions, changing model or effort, toggling workflow automation, and aborting a run may stay as typed presentation coordination without artificial application/domain/effect layers.

## Parity contract for architecture-boundary slice

Readiness gate for #1149/#1219:

- Zero behavior change mandate: passed. The epic explicitly requires “No big-bang rewrite or user-visible behavior changes” and “Existing unit, headless harness, and BDD behavior remains stable.”
- Touched observable surfaces: documentation, executable architecture guardrails, and version strings/metadata for the crates whose files changed. This slice does not move runtime code, rendering code, protocol handling, event handling, or widget state.
- Existing harnesses: passed. The repository has architecture tests, BDD feature execution, TUI unit tests, and TUI render/headless harnesses. This slice pins the documentation/architecture surface with architecture tests and BDD steps; no render surface is touched.
- Behavioral/refactoring split: passed. This slice records feature boundaries and guardrails only; production behavior is unchanged.
- Performance warning: accepted. No specialized runtime/rendering code is replaced in this slice, so there is no old allocation pattern, cache, single-pass path, or complexity characteristic to preserve. Consequence if that premise were wrong: allocation/complexity regressions would be undetectable; later code-moving slices must record performance characteristics for the specific code they replace.

Observable surfaces and required parity:

| Surface | Required identical behavior | Boundary cases | Performance characteristics |
|---|---|---|---|
| TUI runtime behavior | No runtime behavior changes except the intentional patch-version string update required by repository release policy: no command ordering, protocol handling, event routing, terminal handling, render output other than version text, or widget behavior changes. | Not applicable to this docs/guardrail slice; no runtime call sites are changed. | Not applicable; no specialized runtime code is replaced. |
| TUI public crate shape during migration | Current compatibility modules remain available: `application`, `domain`, `infrastructure`, and `interface`; `main.rs` remains a thin entrypoint. | Empty/one/many module cases are not relevant because the compatibility set is exact and already pinned by existing architecture tests. | Not applicable; module exports are compile-time structure. |
| Architecture documentation | The current architecture direction is discoverable from the TUI README; the superseded Clean Architecture target model clearly points to the feature-oriented architecture document; the new document lists all target harness-facing capability modules. | Full target set must be present: shell, protocol, conversation, sessions, agents, workflow, inference, workspace, components. | Not applicable; documentation-only surface. |
| Executable guardrails | Architecture tests and BDD steps continue to execute, and the new feature-oriented guardrail is additive rather than weakening existing interim compatibility checks. | Existing checks still cover the full compatibility layer set and root file placement; the new check covers the full capability list plus protocol and pure-policy boundary rules. | Not applicable; test/runtime cost is outside shipped TUI behavior. |

Approved parity contract: all readiness-gate items are resolved; no `__UNRESOLVED__` markers remain.

## Parity contract for protocol-boundary slice (#1220)

Readiness gate for #1220:

- Zero behavior change mandate: passed. #1220 states “Existing behavior and command ordering remain unchanged”, and the parent epic forbids user-visible behavior changes.
- Touched observable surfaces: enumerated below. The slice moves `list_models` payload interpretation from `interface/app_models.rs` into an application-layer mapper, documents the mapper convention, and adds two decrease-only guard ratchets. No command is added, removed, or reordered.
- Existing harnesses: passed. `app_models_tests.rs` pins the current model-list parsing behavior, `session_payloads_tests.rs` pins the existing mapper style, and `tests/architecture.rs` runs in the fast pre-commit guard suite.
- Behavioral/refactoring split: passed. Every acceptance criterion is structural (mapper convention, mapper tests, ratchets, inventory) or explicitly parity-only.
- Performance warning: accepted. The replaced parsing code is a single-pass `filter_map` over the `models` array allocating one `Vec<ModelEntry>` plus per-entry sanitized `String`s. Consequence if unrecorded: allocation/complexity regressions would be undetectable; the mapper therefore keeps the same single pass and the same allocation count.

Observable surfaces and required parity:

| Surface | Required identical behavior | Boundary cases | Performance characteristics |
|---|---|---|---|
| `list_models` response handling | Model entries are derived from `models[]`; `model` wins over `id`; empty/non-string ids are skipped; provider is explicit value, else slash prefix, else `Model`; `auth` is sanitized and dropped when empty; control characters are stripped from id/provider/auth; `is_current` starts `false`. | Empty array → no entries; missing `models` key → no entries; `models` not an array → no entries; one entry; many mixed entries; id without slash; explicit provider overriding slash inference. | One pass over the array, one output `Vec`, no extra clones beyond existing sanitized strings. |
| Model selector opening | `open_pending` still clears exactly once, the selector still opens after a payload arrives, and a `None` payload still opens the selector with cached entries. | Payload absent; payload present with zero entries; payload present with entries. | Unchanged; no additional cloning of the cached entry list. |
| Command ordering | `ListModels`, `SetModel`, and sub-agent routing emit the same commands in the same order. | Master session vs focused sub-agent; unavailable sub-agent path. | Unchanged. |
| Guard suite | The pre-commit fast guard suite still runs `tests/architecture.rs`, and the two new ratchets fail on increase and pass on decrease. | Count equal to seed; count below seed; count above seed. | Not applicable; guard-time only. |

Approved parity contract: all readiness-gate items are resolved; no `__UNRESOLVED__` markers remain.

## Characterization mutation log for protocol-boundary slice (#1220)

Every mutation was applied to production parsing/dispatch code in `interface/app_models.rs`, verified to fail the named characterization test, then reverted from a pristine baseline copy.

| Mutation | Observed failure |
|---|---|
| M1: prefer `id` over `model` for the entry id | `model_field_wins_over_id_field` FAILED |
| M2: drop `sanitize_control` on the model id | `control_characters_are_stripped_from_rendered_fields` FAILED |
| M3: remove the empty-id skip | `empty_model_id_entry_is_skipped_but_siblings_render` FAILED |
| M4: remove the empty-auth `filter` | `empty_auth_string_yields_no_auth_value` FAILED |
| M5: ignore the explicit `provider` field | `explicit_provider_overrides_slash_inference` FAILED |
| M6: reverse parsed entry order | `renders_model_field_entries_in_payload_order` FAILED |
| M7: change the slashless provider default to `Unknown` | `provider_defaults_to_model_label_without_slash` FAILED |
| M8: never clear `open_pending` | `pending_open_flag_clears_exactly_once` FAILED |
| M9: clear cached entries on an absent payload | `absent_payload_opens_selector_and_keeps_cached_entries` FAILED |
| M10: open the selector for unsolicited lists | `delivery_without_pending_open_updates_cache_without_opening` FAILED |
| M11: substitute a fallback entry when `models` is missing/non-array | `missing_models_key_yields_no_entries`, `models_not_an_array_yields_no_entries` FAILED |
| M12: remove the `id` fallback | `falls_back_to_id_when_model_field_absent` FAILED |
| M13: coerce non-string ids instead of skipping | `non_string_model_id_entry_is_skipped` FAILED |
| M14: delete the slash-prefix provider inference branch | `provider_is_inferred_from_slash_prefix` FAILED |
| M15: parse entries with `is_current: true` | `provider_is_inferred_from_slash_prefix` FAILED |
| M16: drop the `open_pending` re-entrancy guard in `open_model_selector` | `second_open_while_pending_emits_no_duplicate_list_models` FAILED |
| M17: never emit the `ListModels` command on open | `open_model_selector_emits_exactly_one_list_models` FAILED |
| M18: send a fixed wrong model id in `SetModel` | `selector_selection_emits_set_model_command` FAILED |
| M19: remove the empty-id skip (sibling-identity assertion) | `empty_model_id_entry_is_skipped_but_siblings_render` FAILED |

Hollow assertions found and fixed by mutation testing:

- `empty_auth_string_yields_no_auth_value` originally compared rendered frames, which are identical for empty vs absent auth; M4 survived. Rewritten to assert the parsed `auth` value, after which M4 fails.
- `provider_defaults_to_model_label_without_slash` originally matched `"Model"` in the frame, which also matches the selector title `Select Model`; M7 survived. Rewritten to assert the parsed `provider` value, after which M7 fails.

Hollow assertions found by the pre-refactor falsifiability/coverage finders and fixed before the freeze:

- `provider_is_inferred_from_slash_prefix` matched `"anthropic"` in the frame, but the row label IS the id, so M14 survived. Rewritten to assert the parsed `provider`, `id`, and `is_current`; M14 and M15 now fail.
- `empty_model_id_entry_is_skipped_but_siblings_render` used `contains("valid/model") || contains("model")`; the `||` arm matched the empty-state text `No matching models`, making it a tautology. Narrowed and given an entry-identity assertion; M19 now fails.
- `absent_payload_opens_selector_and_keeps_cached_entries` had a latent `|| contains("entry")` arm; narrowed to `cached/entry`.
- `control_characters_are_stripped_from_rendered_fields` asserted `contains("model")`, also satisfied by `No matching models`; narrowed to the full sanitized id `provider/model`.
- `non_string_model_id_entry_is_skipped` asserted only the entry count; given a surviving-entry identity assertion.

Coverage gaps closed before the freeze (command-emission surface, previously pinned only by existence):

- `open_model_selector_emits_exactly_one_list_models` (M17)
- `second_open_while_pending_emits_no_duplicate_list_models` (M16)
- `selector_selection_emits_set_model_command` (M18)

Mutation residue check: `git diff quecto-tui/src/interface/app_models.rs` shows only the added characterization test-module declaration; the suite is GREEN at 20 passed.

Freeze manifest (characterization suite is READ-ONLY from here until the parity step):

| File | `git hash-object` |
|---|---|
| `quecto-tui/src/interface/app_models_protocol_characterization_tests.rs` | `d37d0ad0f0f19fb40bba8f14e6029a285be4d219` |

## Characterization mutation log for architecture-boundary slice

- Capability-list assertion: replacing every `agents` token in this document with `children` made `cargo test -p quecto-agentic-harness --test architecture tui_feature_oriented_architecture_is_documented` fail with missing `"agents"`.
- Protocol-boundary assertion: replacing every `Raw UDS framing` and `raw JSON interpretation` phrase in this document made `cargo test -p quecto-agentic-harness --test architecture tui_feature_oriented_architecture_is_documented` fail at the protocol-boundary requirement.
- Superseded-document assertion: replacing the first `SUPERSEDED` marker in `clean-architecture-target-model.md` with `HISTORICAL` made `QUECTO_TAG=issue-1149 cargo test -p quecto-agentic-harness --features test-support --test bdd` fail at the superseded target-model step.
- README pointer assertion: replacing the first `docs/feature-oriented-presentation-architecture.md` link in `README.md` made `QUECTO_TAG=issue-1149 cargo test -p quecto-agentic-harness --features test-support --test bdd` fail at the README pointer step.
- Mutation residue check: each mutation was restored from `HEAD`; the final architecture and BDD characterization commands passed.

## Characterization freeze manifest

The characterization suite is frozen after review-finder fixes. Until parity verification, these files are read-only unless a logged edit is re-run through mutation evidence:

| File | `git hash-object` |
|---|---|
| `quecto-agentic-harness/tests/architecture.rs` | `cf3fea4ed41c2381925f5f8db8e6e4e381e3a48f` |
| `quecto-agentic-harness/tests/bdd/tui_architecture_steps.rs` | `47caadc63192628a6c5738cdd8ba4be2aff780ce` |
| `quecto-agentic-harness/tests/features/tui_clean_architecture.feature` | `b331f192c2be90bc567932f82efab696c0637b44` |

Review-finder outcomes:

- Refactor-specialized review fix: the architecture and BDD capability-list checks now require exact target bullets such as ``- `agents`:`` instead of unordered whole-document substrings. Mutation evidence was re-run by deleting the `agents` target bullet and confirming `cargo test -p quecto-agentic-harness --test architecture tui_feature_oriented_architecture_is_documented` fails.
- Falsifiability: accepted concerns about source-text/self-asserted checks where applicable; kept document-content characterization because this slice's observable surface is documentation/guardrails, and added/re-ran mutation evidence for the README pointer and protocol boundary.
- Coverage: accepted missing README coverage and added architecture/BDD assertions for the README pointer.
- Gherkin discipline: accepted overclaim/checklist concerns by introducing a `Given` for document presence and narrowing the capability-list step wording to “each target harness-facing capability module.”

## Deletion ledger for architecture-boundary slice

No production runtime code, rendering code, protocol code, or characterization tests were deleted in this slice. The only deletions/replacements are documentation and BDD wording updates:

| Deleted/replaced text | Invariant it enforced | New location preserving invariant |
|---|---|---|
| README phrase “Clean Architecture layering” | The TUI has an explicit architecture direction before `1.0`. | README now names feature-oriented presentation boundaries and links this document. |
| Feature title/body wording that described Clean Architecture as the target | The TUI architecture rules remain executable through BDD. | Feature now describes interim compatibility layers plus the feature-oriented target boundary scenario. |
| Old target-model note saying it would be rewritten or removed under #1219 | Readers must not implement against the abandoned four-layer target. | Superseded banner now links this current feature-oriented architecture document. |

No shared helper was introduced, so consolidation completeness is not applicable for this slice.

## Parity evidence for architecture-boundary slice

| Surface | Behavior | Evidence | Verdict |
|---|---|---|---|
| TUI runtime behavior | No command ordering, protocol handling, event routing, terminal handling, render output other than required version text, or widget behavior changes. | `git diff HEAD~1..HEAD -- quecto-tui/src` is empty; no production TUI source files changed. Patch-version surfaces changed intentionally because changed crates are versioned in this repository. `cargo check -p quecto-tui` passed. | PASS |
| TUI public crate shape during migration | Compatibility modules and thin entrypoint remain available. | `cargo test -p quecto-agentic-harness --test architecture` passed, including existing TUI layer/root-file checks. | PASS |
| Architecture documentation | README points to current feature-oriented doc; old Clean Architecture target is superseded; all target capability modules and boundary rules are documented. | `cargo test -p quecto-agentic-harness --test architecture tui_feature_oriented_architecture_is_documented` passed; `QUECTO_TAG=issue-1149 cargo test -p quecto-agentic-harness --features test-support --test bdd` passed. | PASS |
| Executable guardrails | New feature-oriented guardrail is additive and existing interim compatibility checks still run. | Full architecture integration test passed; BDD target compiled with `cargo test -p quecto-agentic-harness --features test-support --test bdd --no-run`. | PASS |
| Frozen characterization suite | #1219 intentionally extends the architecture guardrail, so `architecture.rs` is not claimed frozen for this slice; the unchanged BDD feature and step files remain the #1149 characterization baseline. | `cargo test -p quecto-agentic-harness --test architecture tui_feature_oriented_architecture_is_documented` passed, including the exact production-file/map comparison added for #1219. | PASS |
| Visual parity | No render frames or visual surfaces are touched. | No files under `quecto-tui/src` changed; this slice only changes docs, tests, Cargo metadata, and lockfile. | PASS |
| Performance parity | No specialized runtime/rendering code is replaced, so no allocation, cache, single-pass, or complexity characteristic changes. | No production code changed. Targeted clippy passed: `cargo clippy -p quecto-agentic-harness --test architecture -- -D warnings` and `cargo clippy -p quecto-tui --all-targets -- -D warnings`. | PASS |
| Quantitative criteria | The issue asks for a net LOC decrease over the broader refactor epic; this slice deliberately adds architecture contract documentation and guardrails. | PR #1228 review-time shortstat after finder fixes: 8 files changed, 179 insertions(+), 9 deletions(-). Recorded as a review-time epic metric, not a test assertion. | PASS for slice / epic metric pending later code-moving slices |

Additional clean checks for this parity pass:

- `cargo fmt --all -- --check`
- `cargo clippy -p quecto-agentic-harness --test architecture -- -D warnings`
- `cargo clippy -p quecto-tui --all-targets -- -D warnings`
- `cargo test -p quecto-agentic-harness --test architecture tui_feature_oriented_architecture_is_documented`
- `QUECTO_TAG=issue-1149 cargo test -p quecto-agentic-harness --features test-support --test bdd`
- `cargo test -p quecto-agentic-harness --test architecture`
- `cargo test -p quecto-agentic-harness --features test-support --test bdd --no-run`
- `cargo check -p quecto-tui`

## Capability characterization and migration map

This issue is the characterization-readiness slice for the later code-moving issues. It does not move production files; it records the owner each current production file should converge toward and the coverage gaps that must be closed before that owner moves code.

### Characterization coverage by capability

| Capability | Existing coverage signals | Gaps to close before moving affected code |
|---|---|---|
| `shell` | TUI event-loop, disconnect, terminal restore, idle-efficiency, stdin, CLI, and headless harness tests. | Before thinning `App`, add characterization around top-level routing ownership, task/channel lifetimes, and cross-feature coordination paths that currently live implicitly in `App`. |
| `protocol` | UDS client unit tests, client defence tests, sync/legacy coverage, paged-history protocol coverage, and BDD UDS-client defence scenarios. | #1220 must pin typed mapper behaviour for every raw JSON shape before controllers/views stop interpreting it ad hoc. |
| `conversation` | Chat/session unit tests, paged-history tests, resumed-history/recovery tests, rewind/response tests, streaming stability BDD, chat render/cache BDD. | Before #1221 moves state, pin request correlation, pagination invalidation, retry, transcript reconstruction, stub recall, and turn recovery as pure policy where invariants are client-owned. |
| `sessions` | Chat session tests, new/reset context BDD, cold-start and foundation BDD, session payload parsing tests. | Add focused coverage for resume/new/clear/stats projections once #1220 mapper types exist. |
| `agents` | Subagent feed/panel/state/stream/roster authority tests, ledger sync tests, subagent parity/read-only/first-layout BDD. | Before #1222 extraction, pin source-scoped roster authority, lifecycle retention, focus preservation, child feed epoch/revision transitions, and ledger sync conflict handling. |
| `workflow` | Workflow bar unit tests, workflow sticky tests, workflow box width tests, workflow-state harness coverage. | Add projection/control characterization for workflow automation toggles and stale workflow-state suppression before moving controls out of `App`. |
| `inference` | Model selector, effort selector, model focus, effort tests. | Add mapper-backed coverage for model discovery/fallback and effort changes once #1220 provides typed feature mapping. |
| `workspace` | Git tests, workspace file tests, autocomplete/file mention BDD and unit tests. | Before moving workspace coordination, pin Git context refresh, ignored-file handling, file preview boundaries, and autocomplete source precedence. |
| `components` | Widget unit tests plus render/cache/list/markdown/table/spacing BDD coverage. | Before relocating `interface/components/` to top-level `components/`, run the existing render characterization and add missing edge cases for any widget whose public module path changes. |

### Dependency rules for migration

- `protocol` owns raw UDS framing, deserialization, raw JSON interpretation, and DTO-to-feature mapping.
- Feature modules own presentation state and coordination for their capability; they may depend on typed protocol results and reusable `components`.
- `shell` owns terminal/runtime resources, Tokio task/channel supervision, top-level event routing, and cross-feature composition.
- `workspace` owns filesystem/Git adapters used by presentation flows.
- Pure policy extracted from a feature must not depend on terminal/widget types, Tokio handles/channels, concrete clients, filesystem/process APIs, or raw JSON.
- Simple presentation flows must not grow mandatory domain/application/effect/port layers; introduce those seams only for a demonstrated invariant, state machine, or dependency inversion need.
- During migration keep one source of truth for each state cluster; avoid dual writes between `App` and a new module.

### Production file target-owner map

| Current production file | Target owner |
|---|---|
| `application/agent_ledger_payloads.rs` | `protocol` mapper feeding `agents` ledger sync (#1222) |
| `application/mod.rs` | remove after compatibility shims are unnecessary |
| `application/model_payloads.rs` | `protocol` mapper feeding `models` |
| `application/range_accumulator.rs` | `protocol` chunked range assembly feeding `conversation` (#1221) |
| `application/session_payloads.rs` | `protocol` mapper feeding `sessions` |
| `domain/history_paging.rs` | `conversation` history cursors, page correlation and backfill latch (#1221) |
| `domain/mod.rs` | remove vestigial placeholder; recreate only for invariant-bearing shared values if needed |
| `domain/turn_recovery.rs` | `conversation` end-of-turn recovery trigger and batch atomicity (#1221) |
| `infrastructure/child_watch.rs` | `shell` runtime supervision |
| `infrastructure/client.rs` | `protocol` |
| `infrastructure/mod.rs` | remove after adapter modules move |
| `infrastructure/process.rs` | `shell` runtime adapter |
| `infrastructure/render.rs` | `shell` terminal/render runtime adapter |
| `infrastructure/signals.rs` | `shell` runtime adapter |
| `infrastructure/terminal.rs` | `shell` terminal adapter |
| `infrastructure/warn_capture.rs` | `shell` diagnostics/runtime adapter |
| `infrastructure/workspace_files.rs` | `workspace` |
| `interface/app.rs` | `shell` composition root plus state delegated to features |
| `interface/app_commands.rs` | `shell` top-level command routing |
| `interface/app_disconnect.rs` | `shell` runtime/disconnect coordination |
| `interface/app_effort.rs` | `inference` |
| `interface/app_event_loop.rs` | `shell` event loop |
| `interface/app_events.rs` | `shell` top-level event routing |
| `interface/app_events_test_support.rs` | `shell` test support |
| `interface/app_git.rs` | `workspace` |
| `interface/app_idle_efficiency.rs` | `shell` event-loop policy |
| `interface/app_inference.rs` | `inference` flow owner |
| `interface/app_ledger_sync.rs` | `agents` |
| `interface/app_message_recovery.rs` | `conversation` |
| `interface/app_methods.rs` | `shell` composition methods until split by feature |
| `interface/app_models.rs` | `inference` |
| `interface/app_paged_history.rs` | `conversation` |
| `interface/app_response.rs` | `conversation` |
| `interface/app_resumed_history.rs` | `conversation` |
| `interface/app_rewind.rs` | `conversation` |
| `interface/app_rewind_state.rs` | `conversation` rewind flow owner |
| `interface/app_selection.rs` | `shell` focus/routing until delegated to feature views |
| `interface/app_sessions.rs` | `sessions` flow owner |
| `interface/app_stdin.rs` | `conversation` input coordination with `shell` stdin adapter |
| `interface/app_subagent_feed.rs` | `agents` |
| `interface/agents/feed.rs` | `agents` pure feed sync state (#1222) |
| `interface/agents/focus.rs` | `agents` focus constants/state (#1222) |
| `interface/agents/ledger.rs` | `agents` pure ledger transcript projection (#1222) |
| `interface/agents/mod.rs` | `agents` module root (#1222) |
| `interface/agents/roster.rs` | `agents` pure roster/lifecycle policy (#1222) |
| `interface/agents/ui.rs` | `agents` concrete UI/runtime adapter state (#1222) |
| `interface/app_subagent_panel.rs` | `agents` |
| `interface/app_subagent_stream.rs` | `agents` |
| `interface/app_subagents.rs` | `agents` |
| `interface/app_submit.rs` | `conversation` |
| `interface/app_workflow.rs` | `workflow` flow owner |
| `interface/app_workspace.rs` | `workspace` flow owner |
| `interface/ansi.rs` | `components` rendering primitive |
| `interface/cli.rs` | `shell` CLI entry |
| `interface/component.rs` | `components` shared traits/primitives |
| `interface/components/autocomplete.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/chat.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/chat_render.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/chat_stub.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/editor.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/effort_selector.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/files_autocomplete.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/footer.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/list_navigator.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/list_rows.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/markdown.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/mod.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/model_selector.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/notification.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/select_list.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/spinner.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/suggestion_list.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/components/workflow_bar.rs` | `components` (relocate physically to top-level `components/`) |
| `interface/fuzzy.rs` | `components` helper used by overlays/autocomplete |
| `interface/keys.rs` | `shell` input mapping primitive |
| `interface/kitty.rs` | `shell` terminal integration |
| `interface/mod.rs` | remove/split into `shell`, features, and `components` |
| `interface/overlay.rs` | `components` overlay primitive |
| `interface/select_overlay.rs` | `components` overlay primitive |
| `interface/stdin_buffer.rs` | `shell` stdin adapter/policy |
| `interface/theme.rs` | `components` styling primitive |
| `interface/tui_harness.rs` | `shell` test harness support |
| `interface/tui_harness_disconnect.rs` | `shell` test harness support |
| `interface/tui_harness_events.rs` | `shell` test harness support |
| `interface/tui_harness_probes.rs` | `shell` test harness support |
| `interface/utils.rs` | split by caller; keep shared UI helpers in `components` |
| `lib.rs` | `shell` crate composition/export root |
| `main.rs` | `shell` thin binary entrypoint |

### Parity contract for agents presentation slice (#1222)

Readiness gate for #1222:

- Zero behavior change mandate: passed. #1222 is explicitly a “zero-behaviour-change refactor” and all acceptance criteria are structural/parity-only.
- Touched observable surfaces: sub-agent roster/lifecycle tracking, source-scoped child roster authority, direct child feed lifecycle and caps, ledger sync epoch/revision/capability transitions, focused-session switching/fallback, retained-session/feed eviction, per-child workflow snapshot stickiness, and synchronized transcript projection. Panel cursor movement, glyphs, layout, elapsed formatting, concrete UDS clients, Tokio tasks, and channel supervision remain presentation/runtime concerns.
- Existing harnesses: passed. The touched surfaces are covered by subagent roster/feed/selection/panel/workflow-sticky tests, ledger-sync tests, subagent parity/read-only/first-layout BDD, and headless TUI harness tests.
- Behavioral/refactoring split: passed. The issue does not request a command, protocol, render, or user-visible behavior change.
- Performance warning: accepted. The replaced specialized code is small BTreeMap/Vec policy over already-owned snapshots and one ledger transcript HashMap/order Vec. Consequence if not recorded: allocation/complexity regressions in roster merge, retained feed eviction, or transcript reconstruction would be undetectable. The extracted policy must keep the same map/vec passes and must not move runtime work into render.

Observable surfaces and required parity:

| Surface | Required identical behavior | Boundary cases | Performance characteristics |
|---|---|---|---|
| Warm sub-agent feed startup and authority | A socket-backed agent gets one warm direct feed with `get_state` then `sync(epoch=0,sinceRev=0)`; capability alone never makes it authoritative; only a valid sync delta promotes authority. | No socket/invalid socket; first feed vs existing feed; sync-capable vs legacy child; capability before/after pending revision. | One feed entry per retained agent up to cap; no reconnect when a feed exists; no render-loop work. |
| Ledger sync state | Ledger hints update epoch/freshness; sync is requested only after capability is known; caught-up deltas clear pending revision; partial deltas continue from `nextRev`; stale epoch deltas without `resync` are ignored; `resync` replaces stale transcript. | Epoch match/mismatch; `resync=true/false`; caught-up vs not caught-up; empty/one/many messages; duplicate ids; stub upsert. | Same O(messages) transcript upsert using one id map plus one order vector; no extra command allocation beyond the existing sync command. |
| Source-scoped roster authority | Master snapshots own roots; direct child feeds own only their source subtree; direct metadata survives later master polls; a child feed cannot hijack/reparent unrelated roots; descendants introduced in one event are accepted recursively. | Empty master snapshot; empty source snapshot; existing root hijack attempt; grandchild discovered with parent in same event; source subtree removal; unknown parent treated as root for display. | Same BTreeMap snapshot merge and ancestor carry-over; no background task or channel in the policy portion. |
| Lifecycle, optimistic spawn, and retention | Running/starting count as active; timers freeze on idle/error/exited and resume on running; exited rows are retained during grace and while siblings are active; unconfirmed optimistic spawns survive omitting snapshots only within grace; confirmed omitted rows drop. | Empty/one/many agents; idle vs running vs exited; expired vs recent exit; active sibling vs quiescent batch; duplicate spawn ToolStart for confirmed id. | Same one-pass status scan for active siblings and retain pass for GC; no extra retained-session/feed capacity. |
| Focus and active-target fallback | `Tab`/panel navigation moves highlight without switching until commit; selecting a child ensures/reuses its session and feed, seeds workflow from snapshot, and refreshes stale authoritative sync; if the active child disappears, focus falls back to master. | Master row; first/last rows; missing selected session; stale authoritative feed; warm unsynced feed; active child removed. | No new allocation on already-existing active-session render path; retained sessions and warm feeds remain bounded by the existing cap. |
| Workflow snapshot stickiness | Per-subagent workflow snapshots remain sticky through workflowless polls and transient empty live events; real progress updates still advance; genuine end/reset clears. | Snapshot `0/N`; transient `0/0`; active issue without real progress; workflowless `get_subagents`; direct `get_state` snapshot. | Same snapshot copy into tracked entry/session only on accepted updates. |
| Synchronized transcript projection | Once a feed is authoritative, legacy live child events no longer duplicate ledger-projected chat, while get-message recovery and deferred notes preserve the current visible transcript behavior. | Authoritative vs warm feed; `get_messages` early return; tool calls/results; user/assistant messages; suppressed tool boxes. | Ledger reconstruction remains one ordered projection over the transcript; no Tokio/runtime types in pure ledger state. |

Review-time structural checks for #1222. These are reviewer-performed
inspections, not executable guardrails: the architecture tests cited below
enforce documentation inventory and heuristic parsing/DTO ratchets only, so
none of the following are mechanically re-checked on later changes.

- Pure agents policy modules contain no Tokio handles/channels, terminal/widget types, concrete client, or raw JSON.
- Runtime feed ownership (task handle and command channel) is separated from feed synchronization state *at construction only*: `FeedState` still stores both flattened, and `App` mutates sync fields through it directly. Nothing prevents a new sync field being added straight to `FeedState`.
- `App` delegates migrated roster/feed/ledger/focus behavior to the agents module without introducing dual writes.
- No deleted child backfill/reconcile path is resurrected.
- Existing subagent, ledger, parity, layout, and read-only tests remain green.

Approved parity contract: all readiness-gate items are resolved; no `__UNRESOLVED__` markers remain.

Deletion ledger for #1222 refactor:

| Deleted/replaced site | Invariant enforced before | New owner / evidence |
|---|---|---|
| `interface/app.rs` inline `SubagentUi` owner state | Sub-agent tracked rows, sessions, feeds, focused pane, direct-event fan-in, and active target live in one owner group. | `interface/agents/ui.rs` owns `SubagentUi`, `SessionView`, concrete feed runtime state, and the runtime-to-chat adapter. Existing `tui_list_render_state` BDD plus `cargo test -p quecto-tui --lib app_subagent_panel_tests` keep the owner-group/session behavior pinned. |
| `interface/app.rs` inline `SessionView` construction | Child/master sessions use identical footer/history/chat/deferred-note initialization. | `interface/agents/ui.rs::SessionView::{new,with_footer}` carries the initializer unchanged; pinned by `app_subagent_panel_tests`, `app_paged_history_tests`, and TUI sub-agent parity BDD. |
| `interface/app.rs` inline `Focus` and panel/retention constants | `Tab` focus model, panel width, and retained-session cap stay stable. | `interface/agents/focus.rs` owns `Focus`, `SUBAGENT_PANEL_WIDTH`, and `MAX_RETAINED_SESSIONS`; pinned by focus parity tests, sub-agent layout BDD, and `retained_sessions_and_warm_feeds_evict_oldest_non_active_beyond_cap`. |
| `interface/app_subagent_state.rs` | Lifecycle status classification, elapsed timer freeze/resume, exited grace GC, optimistic marker retention, and workflow/parent stickiness. | `interface/agents/roster.rs` owns generic `TrackedSubagent`, `RosterInfo`, lifecycle updates, GC and deadline policy. Existing subagent tests plus new characterization tests pin the same lifecycle/retention edges. |
| `interface/app_subagents.rs` hand-rolled source-scoped snapshot merge | Master snapshots own roots; direct feeds own only their source subtree; anti-hijack and recursive descendant acceptance; parent carry-over for surviving descendants. | `interface/agents/roster.rs::apply_roster_snapshot` owns the merge policy. `App::update_subagent_bar_from_source` sanitizes/socket-filters then delegates once. Pinned by `app_subagent_roster_authority_tests`, `app_subagents_tests`, and `active_child_removed_by_its_source_feed_falls_back_to_master_only`. |
| `interface/feed_state.rs` flattened feed sync state | Warm feed is non-authoritative until sync delta; epoch/rev/freshness/capability/pending/transcript fields move together. | `interface/agents/feed.rs::FeedSyncState` owns pure feed synchronization state; `interface/agents/ui.rs::FeedState` separately wraps runtime `cmd_tx`/task handle. `app_subagent_feed.rs` constructs from `FeedRuntime` + `FeedSyncState`; pinned by ledger tests and warm-start characterization. |
| `interface/ledger_sync.rs` raw JSON transcript state | Sync deltas upsert by message id, preserve first order slot, support resync clearing, project user/assistant/tool entries, and parse sync capability. | `application/agent_ledger_payloads.rs` provides typed DTOs; `interface/agents/ledger.rs` owns typed `LedgerTranscript` and pure `LedgerEntry` projection. `interface/agents/ui.rs` adapts `LedgerEntry` to `ChatEntry` at the presentation boundary. Pinned by `ledger_sync_tests` and duplicate-id characterization. |
| Call sites that used old module paths | Same authority/path/type behavior, with no dual writes. | Mechanical path updates point to `interface::agents::{feed,ledger,ui,roster}`. Pre-existing tests were not semantically changed; only expected type/module paths were updated where the extracted owner moved. |

Parity evidence recorded during #1222 implementation:

| Parity class / surface | Behaviour or claim | Evidence | Verdict |
|---|---|---|---|
| Frozen characterization suite | Roster/source authority, warm-feed startup, retained feed/session cap, ledger no-op hints, caught-up sync, duplicate-id transcript upsert, active-child fallback, and authoritative ledger projection preserve behavior. | `cargo test -p quecto-tui --lib app_agents_characterization_tests` passed after refactor. Mutation evidence before freeze killed M1–M15. The ledger projection test needed a mechanical type adaptation from `ChatEntry` to pure `LedgerEntry` after extraction; follow-up mutations M16/M17 against `interface/agents/ledger.rs` both failed the adapted test. | PASS |
| Existing targeted unit suites | Existing subagent, ledger, workflow-stickiness, panel/session, and roster-authority behavior remains unchanged. | Passed: `cargo test -p quecto-tui --lib` (1642 tests); explicit targeted runs of `app_ledger_sync_tests`, `ledger_sync_tests`, `app_subagent_roster_authority_tests`, `app_subagents_tests`, `app_subagent_panel_tests`, and `app_subagent_workflow_sticky_tests`. Existing tests changed only for mechanical module/type moves (`feed_state`/`ledger_sync` into `interface::agents::*`, plus typed `LedgerEntry` projection). | PASS |
| Visual / rendered frames | Panel-first layout, read-only marker, and sub-agent session parity render identical user-visible surfaces. | `cd quecto-tui && QUECTO_TAG=tui cargo test --features test-harness --test bdd` passed 28 TUI features / 175 scenarios, including `tui_subagent_first_layout`, `tui_subagent_readonly_marker`, and `tui_subagent_session_parity`. | PASS |
| Architecture policy | Raw JSON and wire DTO parsing sites do not grow; pure agents policy has no Tokio handles/channels, terminal/widget types, or concrete client; runtime feed ownership remains separated. | `cargo test -p quecto-agentic-harness --test architecture tui_interface_raw_json_parsing_sites_do_not_grow -- --exact` and `cargo test -p quecto-agentic-harness --test architecture tui_wire_dto_usage_does_not_grow -- --exact` passed. `grep` over `interface/agents/{feed,roster,ledger,focus}.rs` found no channels, task handles, concrete client, terminal/widget types, or `serde_json`; only `tokio::time::Instant` remains in roster lifecycle timestamps. Caveat on the ratchets: both counters are heuristic — the raw-JSON counter matches literal `.get("`/`.pointer("` and accessor+`and_then` lines, so `serde_json::from_str`/`from_value` and dynamic `value.get(key)` in `application/agent_ledger_payloads.rs` are not counted; the wire-DTO counter matches `Command::`/`Event::`/`infrastructure::client` lines, so unqualified DTO uses after a single grouped import are not counted. They confirm no growth in the shapes they measure, not the absence of raw JSON parsing. Runtime `cmd_tx`/task handle live in `interface/agents/ui.rs::FeedRuntime`, separate from `FeedSyncState`, but are re-flattened into `FeedState` (see the structural-check caveat above). | PASS |
| Performance parity (source inspection only; no enforcing check) | Replaced specialized code keeps the same asymptotic passes and allocation boundaries. | Roster snapshot merge still builds one `BTreeMap` of candidates/incoming/new map and one parent carry-over queue in `apply_roster_snapshot`; no render-loop work or background task moved into policy. Ledger transcript still uses one `HashMap` plus one ordered `Vec` and applies deltas in O(messages). Retention still evicts through the existing session-order vector and active-id skip. Warm feeds remain bounded by `MAX_RETAINED_SESSIONS`. | PASS |
| Quantitative / formatting / lint | File-size cap and strict local checks remain green. | Largest new agents file is `interface/agents/roster.rs` at 294 lines; `app.rs` dropped from 741 to 617 lines. Overall changed-file line count is +884 (1437 insertions, 553 deletions) including 310 lines of new characterization tests, 203 lines of typed ledger DTOs, and the parity/deletion documentation below. `cargo fmt --check` and `cargo clippy -p quecto-tui --lib -- -D warnings -W clippy::cognitive_complexity -W clippy::too_many_arguments -W clippy::too_many_lines` passed. | PASS |

### Parity contract for final App composition slice (#1224)

Readiness gate for #1224:

- Zero behavior change mandate: passed. #1224 is explicitly a final refactor and architecture-guardrail issue; acceptance criteria are structural/parity-only.
- Touched observable surfaces: App construction, model/effort selector state, resume selector state, workflow automation mirror state, rewind selector/request correlation state, workspace Git/file-autocomplete state, and architecture guard execution. This slice does not change commands, rendering semantics, UDS payloads, FIFO command sending, terminal ownership, or feature policy.
- Existing harnesses: passed. The touched flows are covered by TUI unit tests for model/effort/session/workflow/rewind/workspace paths plus architecture tests.
- Behavioral/refactoring split: passed. No new command, protocol, render, or user-visible behavior is requested.
- Performance warning: accepted. The moved state owners wrap existing fields without replacing specialized algorithms. Consequence if not recorded: construction/allocation regressions in selectors or file autocomplete setup would be undetectable. The extracted owners must keep identical constructors and lazy overlay creation.

Observable surfaces and required parity:

| Surface | Required identical behavior | Boundary cases | Performance characteristics |
|---|---|---|---|
| App construction | App still constructs the same terminal/client/render shell plus the same feature flow state defaults. | Fresh app; default model/effort/session/workflow/rewind state; workspace with and without current Git branch. | Same eager construction of editor/autocomplete/workspace file autocomplete and same lazy selector construction. |
| Inference selector state | Current model/effort, model registry cache, pending model open flag, and effort level vocabulary keep the same update and fallback behavior. | Empty/non-empty registry; pending open; focused child vs master; direct effort vs selector. | Same cached Vec and optional overlay fields; no background task or extra protocol command. |
| Sessions/workflow/rewind flow state | Resume selector, context-stats request latch, workflow auto-continue/completion-nudge mirrors, and rewind request correlation remain single-source state. | Empty/one/many sessions; stale/late rewind responses; toggles true/false. | Same optional selector fields and scalar latches; no duplicate global event hierarchy. |
| Workspace flow state | Git branch footer updates and file autocomplete loading/rendering behave unchanged. | No repo; branch unchanged/changed; autocomplete active/inactive; empty/loaded file list. | Same one FilesAutocomplete with capacity 8 and same asynchronous load trigger. |
| Architecture guardrail | App composes named feature flow owners and does not define those owner structs inline. | All owner fields present; any owner struct definition moved back into app.rs fails. | Guard-time only. |

Approved parity contract: all readiness-gate items are resolved; no `__UNRESOLVED__` markers remain.

Characterization review/freeze manifest for #1224:

- Review finders: falsifiability — no findings; coverage — accepted and fixed missing production file-autocomplete capacity pin and duplicate flattened-state guard; Gherkin — no findings (no BDD changed).
- Mutation log after fixes: changing `FilesAutocomplete::new(8)` to `new(7)` fails `app_workspace_file_autocomplete_uses_production_visible_capacity`; adding a flattened `current_model` field to `App` fails the App owner guard/build; prior mutations renamed `workspace` and reintroduced inline `WorkspaceFlow`, both failed and were reverted.
- Frozen characterization files:
  - `quecto-agentic-harness/tests/architecture.rs` — `2bf1266ee51f0437f989a346f90994d8a50f0377`
  - `quecto-tui/src/interface/app_event_loop_cov_tests.rs` — `9455924255371edce177ebbfa1351c295d50947a`

Deletion ledger for #1224 App thinning:

- Deleted inline `RewindFlow`/fields from `app.rs`: rewind selector, double-Escape timestamp, request ids, and request sequence invariants are re-established unchanged in `interface/app_rewind_state.rs` and accessed through the existing `rewind` owner.
- Deleted inline `SessionsFlow`: resume selector and context-stats request latch invariants are re-established unchanged in `interface/app_sessions.rs`.
- Deleted inline `WorkflowFlow`: auto-continue and completion-nudge mirror invariants are re-established unchanged in `interface/app_workflow.rs`.
- Deleted inline `InferenceFlow`/`ModelRegistry`: current model, selector overlays, registry cache, pending-open latch, current effort, and effort vocabulary invariants are re-established unchanged in `interface/app_inference.rs`.
- Deleted inline `WorkspaceFlow`: file autocomplete capacity 8, Git branch footer state, and Git repo polling root invariants are re-established unchanged in `interface/app_workspace.rs`; production capacity is pinned by `app_workspace_file_autocomplete_uses_production_visible_capacity`.
- No tests were deleted. Frozen characterization tests were not edited after freeze.
- Consolidation completeness: no shared helper was introduced or canonicalized in this slice; moved state owners only.

Parity evidence for #1224:

| Surface | Behaviour/performance/quantity checked | Evidence | Verdict |
|---|---|---|---|
| Formatting and lint | Touched TUI lib and architecture test compile cleanly under strict warnings. | `cargo fmt --check`; `cargo clippy -p quecto-tui --lib --all-targets -- -D warnings`; `cargo clippy -p quecto-agentic-harness --test architecture -- -D warnings`. | PASS |
| Frozen characterization suite | Frozen tests remain unmodified after freeze and green. | Hashes still match freeze manifest: `architecture.rs` = `2bf1266ee51f0437f989a346f90994d8a50f0377`; `app_event_loop_cov_tests.rs` = `9455924255371edce177ebbfa1351c295d50947a`. Targeted frozen tests pass. | PASS |
| Pre-existing targeted behaviour | Touched crates' targeted suites remain green. | `cargo test -p quecto-agentic-harness --test architecture`; `cargo test -p quecto-tui --lib` (1652 passed). | PASS |
| App construction | Same fields and constructor expressions are preserved; only owner type definitions moved. | `App::new` still creates `Autocomplete::new(..., 8)`, `WorkspaceFlow::new(git_branch, git_repo)`, default inference/session/workflow/rewind owners; architecture owner guard passes. | PASS |
| Inference/session/workflow/rewind/workspace state | Each migrated state has one source of truth and no duplicate flattened `App` fields. | Architecture guard `tui_app_state_is_composed_from_feature_flow_owners` checks owner fields and rejects duplicate flattened fields; extracted modules preserve original fields/derives. | PASS |
| Visual parity | No rendering code or pinned frame fixtures changed; owner movement leaves render call sites unchanged. | `cargo test -p quecto-tui --lib` includes pinned list/autocomplete/model/select/workflow/footer/chat render tests; all pass. | PASS |
| Performance parity | No shared replacement or extra pass introduced; hot-path structures keep same lazy/eager construction. | `WorkspaceFlow::new` still constructs one `FilesAutocomplete::new(8)`; model/effort/resume/rewind selectors remain `Option` lazy overlays; no new background task, clone loop, allocation cache, or protocol command was added. | PASS |
| Quantitative | `App` thinned and file cap respected. | `app.rs` is 579 lines; extracted owner modules plus app total 658 lines; production touched Rust diff is 20 added/79 deleted (net -59), TUI interface Rust net -31. | PASS |

### Inter-issue sequencing

1. #1220 establishes protocol mapper conventions and typed feature inputs.
2. #1221 (`conversation`) and #1222 (`agents`) depend on #1220 and may proceed in parallel after it lands.
3. #1223 moves lower-risk sessions, workflow, inference, and workspace presentation flows after the mapper convention is available.
4. #1224 lands last: thin `App`, remove vestigial compatibility modules such as `domain/`, relocate `components`, and enforce final architecture checks.

## Migration sequence

1. Record feature boundaries, dependency rules, and characterization coverage.
2. Establish the typed protocol mapping convention and stop growth of raw JSON in controllers/views.
3. Extract conversation history and recovery.
4. Extract agents roster, feed, ledger, and focus behavior.
5. Modularize sessions, workflow, inference, and workspace presentation flows.
6. Reduce `App` to composition, top-level event routing, runtime ownership, and cross-feature coordination.
7. Tighten architecture checks around meaningful dependency rules after ownership has moved.

## Non-goals

- No big-bang rewrite or user-visible behavior changes.
- No mandatory rich `domain/` or `application/` layer in `quecto-tui`.
- No broad domain vocabulary created in advance.
- No global application event/effect enum mirroring UDS.
- No port-per-command or async-trait framework.
- No direct dependency on private `quecto-agentic-harness` internals.
- No generic history abstraction that erases master-history versus child-ledger differences.
- Do not resurrect deleted child backfill/reconcile paths.
- Do not move runtime or rendering concerns merely to reduce `App` line count.

## Protocol boundary and typed mapper convention (#1220)

Harness-facing features must not interpret raw `serde_json::Value` shapes in
controllers or views. Payload interpretation lives in **application-layer
mappers**; the interface converts a typed DTO into its own view model at a thin
seam. This introduces no global command/event enum mirroring UDS, and no
gateway or port per command.

### The convention

Every mapper in `application/` obeys four rules (stated canonically in the
module docs of `application/model_payloads.rs`):

1. **Input is raw wire JSON, output is a typed application value.** Feature
   controllers and views consume the typed value and never re-read the JSON.
2. **Total, never failing on shape.** Malformed, legacy, and unknown payloads
   map to an empty/defaulted result, unless the distinction is itself
   user-visible (as with `session_payloads::ResumeMessagesError`).
3. **The application layer owns no interface types.** Mappers must not name
   `interface::` types. Where a presentation concern is genuinely needed —
   control-character sanitization, owned by `interface/ansi.rs` — it is
   *injected* by the caller rather than imported, keeping the layer rule intact
   while all derivation rules stay in the mapper.
4. **Parity quirks live in the mapper, documented.** Legacy field fallbacks and
   sanitization rules preserved for zero-behaviour-change parity sit next to the
   canonical rules, never re-implemented at a consuming call site.

### Reference production flow

`list_models` is the first flow migrated to the convention:

- `application/model_payloads.rs` — `parse_model_list` → `Vec<ModelListEntry>`,
  owning the id/provider/auth derivation and the drop rules for absent,
  non-string, and sanitize-to-empty identifiers.
- `interface/app_models.rs::parse_model_entries` — the seam: maps the DTO to the
  selector's `ModelEntry`, adding only the interface-owned `is_current: false`.

Mapper fixtures in `application/model_payloads_tests.rs` cover valid, legacy
(`id` vs `model`), and malformed (missing key, non-array, non-object entries,
non-string ids, control characters) payloads. Observable behaviour and command
ordering are pinned end-to-end by the frozen characterization suite.

### Ratchets

Two decrease-only guards live in `quecto-agentic-harness/tests/architecture.rs`
and therefore run in the fast pre-commit guard suite (targets are enumerated
dynamically, so they cannot be silently dropped):

- `tui_interface_raw_json_parsing_sites_do_not_grow` — interface seed `120`.
- `tui_application_raw_json_parsing_sites_do_not_grow` — application seed `69`.
- `tui_wire_dto_usage_does_not_grow` — seed `124`.

Both were hardened after review on #1235, which proved the first drafts did not
measure what they claimed:

- **Usage, not imports.** The wire-DTO guard originally counted
  `use ... infrastructure::client` lines and saw only 2, because `interface/app.rs`
  imports the DTOs and siblings reach them through `use super::*`. A probe module
  constructing `Command::Prompt` via the glob compiled and left the guard green.
  It now counts `Command::`/`Event::`/`infrastructure::client` *usages*, so globs
  and fully-qualified paths are visible; the same probe now fails it.
- **JSON-aware counting.** `as_str()` also exists on `String`, so the raw-JSON
  count included `match args[i].as_str()` and `m.id.as_str()` and listed modules
  that parse no wire payload at all. An accessor is now counted only alongside a
  key lookup (`.get("…")`/`.pointer("…")`) or an `and_then` chain, and the
  inventory below contains only genuine payload parsers.
- **`cfg(test)`-based exclusion.** Exclusion was by filename (`*_tests.rs`,
  `tui_harness*`), which exempted `tui_harness*.rs` — real production modules
  gated by the `test-harness` *feature*, not `cfg(test)` — while leaving
  `*_test_support.rs` fixtures counted as production. Files are now classified by
  content (whole-file `#![cfg(test)]` or an actual test body); a bare
  `#[cfg(test)]` does not count, since production modules carry one on their
  trailing `mod tests;`.
- **Per-ratchet allowlists.** `interface/app_response.rs` is exempt from the
  raw-JSON ratchet only — it *is* the dispatcher that routes raw responses to
  mappers — and is now measured by the wire-DTO ratchet. The wire-DTO allowlist
  is empty.
- **No vacuous pass.** Both ratchets assert the scan yielded a non-empty file
  list, so renaming the scan root fails them instead of silently disabling them.

Seeds may be lowered as sites migrate; they may never be raised.

### Raw-JSON burn-down inventory

The interface raw-JSON ratchet is seeded at 120 sites (lowered from 130 in #1221 when `range_accumulator.rs` and its 10 sites moved to `application/`). The application raw-JSON ratchet is seeded separately at 69 sites, so the moved wire parsing remains measured instead of disappearing from the burn-down surface. Each ratchet's failure message reprints the live inventory in burn-down order; this document intentionally does not duplicate per-file counts that can go stale.

Wire-DTO usage is seeded at 124, led by `app_subagent_stream.rs`,
`app_subagents.rs`, `app_submit.rs`, and `tui_harness_events.rs`.

### Deletion ledger

| Deleted | Invariant it enforced | Where re-established |
|---|---|---|
| Inline `models` array extraction in `parse_model_entries` | missing/non-array `models` yields no entries | `model_payloads::parse_model_list` early return |
| Inline `model`-over-`id` fallback | legacy payload identifier parity | `parse_model_list_entry`, documented as legacy parity |
| Inline `id.is_empty()` skip | blank rows never render | `parse_model_list_entry` skip, pinned by `identifier_empty_after_sanitization_is_dropped` |
| Inline provider `or_else` slash inference + `"Model"` default | provider grouping label | `parse_model_list_entry`, pinned by mapper tests and M14 |
| Inline auth sanitize + `filter(!is_empty)` | empty auth renders no label | `parse_model_list_entry`, pinned by `empty_auth_is_dropped` |
| `is_current: false` literal | freshly parsed entries are never marked current | retained at the interface seam (interface-owned view concern), pinned by M15 |

No tests were deleted. `app_models_tests.rs` retains its fast pure-function
coverage against the unchanged `parse_model_entries` signature.

Consolidation completeness: `parse_model_list` is the only `list_models` payload
interpreter in the crate — `grep` for `"models"` in production interface code
returns no other hand-rolled parser. The remaining raw-JSON sites in the
inventory above parse *different* payloads and are recorded for burn-down rather
than forced through this mapper.

### Parity evidence (#1220)

| Class | Surface | Evidence | Verdict |
|---|---|---|---|
| Behavioural | frozen characterization suite | `git hash-object` of `app_models_protocol_characterization_tests.rs` is `d37d0ad0…` before AND after the refactor — byte-identical, zero test edits; 20/20 pass | PASS |
| Behavioural | whole TUI crate | `cargo test -p quecto-tui --lib` → 1576 passed, 0 failed | PASS |
| Behavioural | pre-existing `app_models_tests.rs` unit suite | unchanged signature `parse_model_entries(&Value) -> Vec<ModelEntry>`; 46 `app_models` tests pass unmodified | PASS |
| Behavioural | architecture guards | `cargo test -p quecto-agentic-harness --test architecture` → 37 passed | PASS |
| Visual | model selector overlay | frame-level assertions in the frozen suite (provider/auth/id rendering, sanitized output, cached-entry rendering) pass unchanged | PASS |
| Command ordering | `list_models` / `set_model` | `open_model_selector_emits_exactly_one_list_models`, `second_open_while_pending_emits_no_duplicate_list_models`, `selector_selection_emits_set_model_command` all GREEN; `open_model_selector`/`send_set_model` bodies untouched by the refactor | PASS |
| Performance | payload parsing | Old code: one `filter_map` pass over `models`, one output `Vec`, `sanitize_control` allocating per field. New code: identical single `filter_map` pass, one output `Vec`, same per-field sanitization. The seam adds one `into_iter().map()` that moves `String` fields into `ModelEntry` — no clones, no second parse. Parsing runs once per `list_models` response, not per keystroke or per frame. | PASS (no regression) |
| Performance | dispatch | `sanitize` is passed as `&dyn Fn`, adding one indirect call per field on a response-rate path (a few dozen calls per selector open). Chosen over a generic parameter to keep the mapper object-safe and non-monomorphized; cost is immeasurable at this call rate. | PASS (accepted, documented) |
| Quantitative | `app_models.rs` | 33 lines deleted, 18 added → net −15 lines at the interface call site; raw-JSON parsing sites in that file drop to 0 | RECORDED |
| Quantitative | ratchet seeds | raw-JSON sites `173`, wire-DTO imports `2` — both measured, not estimated | RECORDED |

Mutation re-verification after the refactor confirms the pins still bind to the
relocated logic: M14 (drop slash inference), M15 (`is_current: true`), M16 (drop
pending guard), M17 (never emit `ListModels`), M18 (wrong `set_model` id), and
M19 (drop empty-id skip) each fail their named test, with no residue.

### Review response (#1235)

| Finding | Disposition |
|---|---|
| Wire-DTO ratchet counted `use` lines, evadable via `use super::*` (proven by a compiling probe) | FIXED — counts usages; the probe now fails the guard |
| Raw-JSON predicate type-blind, inventory listed non-JSON modules | FIXED — accessors require a key lookup or `and_then`; seed 173 → 130, inventory now only real parsers |
| Ratchets pass vacuously if the scan root is renamed | FIXED — both assert a non-empty file list; verified by renaming the root |
| Filename-based exclusion exempted feature-gated production (`tui_harness*`) and counted `*_test_support.rs` | FIXED — content/`cfg(test)`-based classification |
| Allowlist exemption broader than its rationale | FIXED — per-ratchet allowlists; `app_response.rs` now measured by the wire-DTO guard |
| Mapper test double weaker than the real sanitizer (ANSI/bidi) | FIXED — documented on the double, plus `ansi_and_bidi_controls_are_stripped_through_the_mapper` running the real sanitizer end-to-end (killed by M20, an injected identity sanitizer) |
| `quecto-tui/README.md` version stale at 0.70.13 | FIXED — now current |
| "Three surviving hand-rolled model-payload parsers" in `app_response.rs`, `app_subagent_stream.rs`, `footer.rs` | **DECLINED** — those read a top-level scalar from a `set_model`/`get_state` response (`get("model") → as_str → sanitize`): no array, no `id` fallback, no empty-skip, no provider inference, no auth. They are not `list_models` interpreters. Folding them into the mapper would pull `sanitize_control` back into `application/`, violating rule 3. They remain recorded in the burn-down inventory. |

The characterization suite was re-frozen after adding the ANSI/bidi test (an
additive pin; no existing assertion was altered).

| File | `git hash-object` |
|---|---|
| `quecto-tui/src/interface/app_models_protocol_characterization_tests.rs` | `e57d8c569df490354949ff259b450eb050e661d0` |

## Parity contract for conversation history/recovery slice (#1221)

Readiness gate for #1221:

- Zero behaviour change mandate: **passed**. Every acceptance criterion is structural (testability without terminal/client/JSON/Tokio, `ChatEntry` stays a view projection, `App` delegates, existing tests stay green), and the "Preserve" section enumerates behaviours that must be identical. The parent epic (#1149) forbids user-visible behaviour changes.
- Touched observable surfaces: enumerable — master attach backfill, older-page pagination/correlation/retry, resume/rewind transcript replacement, lifecycle invalidation, stub recall (#1061), ref-based end-of-turn recovery (#1060), and chunked range assembly. No command is added, removed, or reordered.
- Existing harnesses: **passed**. `app_paged_history_tests.rs` (638 lines), `app_paged_history_review_tests.rs` (317), `app_attach_backfill_tests.rs` (417), `app_events_1060_tests.rs`, `app_events_1060_child_tests.rs`, `app_rewind_response_tests.rs`, `app_resumed_history`-covering tests, plus the `TuiHarness` headless harness and BDD features pin the current behaviour without a real terminal.
- Behavioural/refactoring split: **passed**. No acceptance criterion changes rendered output or the wire.
- Performance warning: **accepted**. Recorded per surface below. Consequence if unrecorded: allocation/complexity regressions would be undetectable.

Observable surfaces and required parity:

| Surface | Required identical behaviour | Boundary cases | Performance characteristics |
|---|---|---|---|
| Older-page request emission (`next_history_page_request`) | A `get_messages{before}` is emitted only when: no sub-agent is focused, `history_has_more_before` is true, the chat is scrolled to the oldest loaded entry, and a `history_before_cursor` exists. Request id is `history-page-{uuid_like}-{seq}`, `seq` incremented with wrapping add before use. Same-cursor in-flight requests are deduped unless older than 30s (`PENDING_HISTORY_PAGE_RETRY`), in which case a fresh request replaces the pending entry. | No cursor → no request; `has_more_before=false` → no request; not at oldest → no request; sub-agent focused → no request; pending same cursor fresh (<30s) → suppressed; pending same cursor stale (≥30s) → re-issued; `seq` at `u64::MAX` wraps. | One `format!` per request, one clone of the cursor; no per-frame allocation — the request path is scroll-driven, not render-driven. |
| Page correlation (`is_pending_history_page`) | Only an EXACT `request_id` match is accepted; `None` ids never match; ids with a matching prefix but different suffix are rejected; foreign/broadcast `history-page-*` responses are dropped without mutating chat; a page in flight across a resume is dropped. | `id=None`; exact match; prefix-only match; foreign `history-page-…`; id-less broadcast snapshot (goes to the attach-backfill reconcile path). | Single `Option<&str>` comparison, no allocation. |
| Backfill reconcile (`reconcile_master_backfill_history`) | Returns immediately when `history_backfilled`; unparseable payload is ignored; cursors (`history_before_cursor`, `history_has_more_before`) and `history_pending_page=None` are set BEFORE the empty-history early return; an empty/filtered history never latches `history_backfilled`; `extend_prefix=true` prepends and grows `partial_backfill_len` by the page length; `extend_prefix=false` with an existing `partial_backfill_len` replaces the whole loaded prefix; otherwise prepends; `trimmed || has_more_before` keeps `partial_backfill_len=Some(loaded_prefix)` and `history_backfilled=false`, else clears the prefix and latches `history_backfilled=true`. | Zero messages; one; many; `trimmed=true` with `has_more_before=false` and vice versa; missing `before`/`hasMoreBefore` keys; repeated snapshot after a partial page (no duplicate newest slice, no interior gap). | One `Vec<ChatEntry>` per payload, one prepend/replace pass; no recompute of already-loaded entries. |
| Lifecycle invalidation (`clear_message_recovery`) | Clears recovery batches, pending recoveries, pending stub recalls, and failed stub recalls, AND resets `history_pending_page`, `history_before_cursor`, `history_has_more_before`, `partial_backfill_len`, `history_backfilled` on the master session. Called on resume, rewind ack, clear_history, and legacy replacement. | Empty state (idempotent); state populated in every map. | Map `clear()` only; capacity retained. |
| Zero-fetch / replacement paths | `is_history_page_payload` requires a `messages` array AND (`hasMoreBefore` bool OR a `before` key); paged payloads go through `replace_master_chat_with_history_page` (clear chat, reset backfill flags, reconcile cursors, then append the `Status` line); legacy payloads keep the wholesale-replacement path with the same status text (`Session resumed` / `Conversation rewound`). | `messages` present without either cursor key → legacy; `hasMoreBefore` present but non-bool with `before` absent → legacy; `before: null` present → paged. | Predicate is two key lookups, no parse. |
| Stub recall (#1061) | Visible stub ids are fetched at most once each while in flight (deduped by `(agent_id, message_id)`), skipped when in `failed_stub_recalls`; request id `stub-recall-{uuid_like}`; sub-agent stubs are fetched through the MASTER connection carrying the child's agent id; failure, absent data, `id` mismatch, unknown session, or role mismatch marks the pair failed (no retry per scroll); multi-page bodies continue via `RangeAccumulator` with `offset`/`limit=GET_MESSAGE_PAGE_BYTES`; the assembled body is control-sanitized ONCE after reassembly; a failed `recall_stub` marks the pair failed. | Zero visible stubs; one; many; already-pending; already-failed; role mismatch; missing session; single-page vs multi-page body; sanitization of a control sequence split across pages. | One request per stub, `GET_MESSAGE_PAGE_BYTES = PROTOCOL_LINE_CAP_BYTES/4` per page; content accumulated in a single `String` via `push_str` (no per-page re-concat); sanitize runs once. |
| Ref-based turn recovery (#1060) | Trigger rule unchanged: empty/ellipsis assistant text, or `assistant_text.len() < expected_content_len`, or `refs.len() != tools_this_turn*2+1`; `open_tool_calls > 0` FORCES recovery regardless; empty refs never trigger; refs already in flight are skipped (no double-fetch); a batch spans `[active_turn_start.min(entry_count), entry_count)`; the range is replaced ATOMICALLY only once all refs have responded; failure/absent data/`id` mismatch/range error abandons the whole batch (chat untouched); a batch whose target session vanished is dropped. | Zero refs; one ref; refs count exactly `2n+1` vs off by one; `expected_content_len` absent vs greater vs equal; open tool call with otherwise-satisfied heuristics; late response after batch removal. | One `HashMap` of responses per batch, one `Vec<ChatEntry>` built at completion, exactly one `replace_range` (no incremental splices). |
| Transcript assembly (`recovered_chat_entries` / `resumed_chat_entries`) | Ordering, suppressed-tool-call handling, tool-result attachment to the matching pending call (first `result: None` match), standalone tool results, unknown roles ignored, empty user content skipped, control sanitization on resumed text/args/tool names, and stub-vs-plain entry selection (`history_entry`: stub only when demoted AND id present) all identical. | Empty refs/messages; suppressed call plus its result; result without a call; duplicate call ids; missing `role`/`content` keys; `toolCalls` vs `tool_calls`, `toolCallId` vs `tool_call_id`, `isError` vs `is_error`. | Single pass over refs/messages with an index map; entries pushed once, results patched in place. |
| Range assembly (`RangeAccumulator`) | Offset mismatch, missing content, missing `nextOffset`, non-progressing/overshooting offsets, and length mismatch all error exactly as today. | `offset` absent (defaults 0); `contentLength` absent (defaults to accumulated len); `nextOffset == offset`; `nextOffset > contentLength`; accumulated length over/under `contentLength`. | Amortized `push_str` into one buffer. |
| Command ordering / FIFO | The same commands are emitted in the same order for attach, scroll-back, stub recall, resume, rewind open/apply/refresh, and recovery. Failed enqueues still roll back the matching pending page or stub recall (`rollback_failed_history_command`). | Rollback for a `GetMessages` id that is not the pending page (no-op); rollback for an unknown `GetMessage` id (no-op). | Unchanged. |

Structural goals recorded as REVIEW-TIME checks only (never test assertions): a cohesive conversation module owns the above policy; `ChatEntry` remains a view projection, not policy vocabulary; `App` delegates rather than implements; policy is constructible without a terminal, concrete client, raw JSON, or Tokio runtime.

Approved parity contract: readiness gate passed, all boundary cases verified against the current code, no `__UNRESOLVED__` markers remain.

## Characterization mutation log for conversation history/recovery slice (#1221)

Each mutation was applied to production code, verified to fail the named
characterization test, then restored from a pristine baseline copy.

| Mutation | Observed failure |
|---|---|
| M1: drop the `history_has_more_before` guard on page emission | `scroll_back_emits_no_page_request_when_no_older_history_is_advertised` FAILED |
| M2: default a missing `before` cursor to `""` instead of bailing | `scroll_back_emits_no_page_request_without_a_cursor` FAILED |
| M3: correlate pages by id PREFIX instead of exact match | `page_response_with_prefix_matching_id_is_rejected` FAILED |
| M4: publish paging cursors AFTER the empty-history early return | `empty_backfill_page_still_publishes_its_paging_cursor` FAILED |
| M5: widen `is_history_page_payload` to accept any `messages` array | SURVIVED — the predicate is not the observable; replaced by M5b |
| M5b: route legacy resume payloads through prepend/reconcile instead of replacement | `legacy_resume_payload_without_paging_metadata_replaces_transcript` FAILED |
| M6: accept a stub-recall body whose `id` disagrees with the request | `stub_recall_rejects_mismatched_body_id_and_does_not_retry` FAILED |
| M7: ignore `contentLength` in the recovery trigger heuristic | `truncated_assistant_body_triggers_recovery_via_advertised_content_length` FAILED |
| M8: apply a recovery batch before all refs have responded | `partially_answered_recovery_batch_does_not_mutate_the_transcript` FAILED |
| M9: make `rollback_failed_history_command` a no-op for `GetMessages` | `failed_page_enqueue_rolls_back_the_pending_request` FAILED |
| M10: roll back the pending page for ANY `GetMessages` id | `rollback_of_an_unrelated_page_id_is_a_no_op` FAILED |

Residue check after the log: `git diff` shows only the characterization-test
module declaration and this document; no production code changes.

## Characterization review outcome (#1221)

Three independent read-only finders reviewed the safety net: falsifiability,
contract coverage, and Gherkin/BDD discipline.

Falsifiability (0 HIGH, 4 MED): no hollow assertions, no banned patterns, and the
mutation log — including M5's `SURVIVED` verdict — was independently confirmed
honest. All four MED findings fixed: rollback tests now drive the real
`handle_command_send_failure` entry point rather than the rollback helper alone,
and positive/negative controls were added for the no-retry, batch-atomicity and
`contentLength` assertions so none can pass vacuously.

Coverage — two HIGH gaps found and closed:

- `RangeAccumulator` had NO direct tests; all five `RangeError` variants were
  unpinned even though both callers map `Err(_)` onto a terminal outcome
  (stub marked failed / batch abandoned), so a misclassification silently drops
  user content. Added `range_accumulator_tests.rs` (11 tests) covering every
  variant, both documented defaults, and in-order multi-page reassembly.
- The multi-page `Continue` arm in `app_message_recovery` — duplicated paging
  logic — was never exercised. Added
  `oversized_recovery_ref_pages_and_reassembles_before_replacing_the_turn`.

MED gaps also closed: control-sequence sanitization across a split recall page
boundary, in-flight stub-recall dedupe, and the trimmed-plus-more-history latch.

BDD (2 HIGH, both genuine and fixed):

- Two oversized-recall `Then` steps compared two TEST-LOCAL fixtures to each
  other (`recalled_full` vs `stub_full`), asserting nothing about the app. A
  `RangeAccumulator` mutation that corrupted every reassembled body passed them.
  They now observe the app's real transcript via a new `master_assistant_texts`
  harness probe (the rendered frame is width-wrapped, so an oversized body
  cannot be substring-matched against it), and the mutation now fails them.
- "the operator resumes the session in the TUI" performed no resume: it injected
  a hand-written `resume-messages` id, leaving resume request emission and
  correlation unexercised. It now acknowledges a `resume_session` and replies
  using the id the APP emitted; deleting the production request now fails it.

Also removed four orphaned step definitions for sub-agent paging. Restoring their
scenario proved sub-agent paging was deliberately REMOVED in #1210
(`next_history_page_request` returns `None` when a sub-agent is focused), so the
dead steps were deleted rather than the scenario revived.

Net: 1602 unit tests and 192 BDD scenarios green; the only production-source
changes are the test-module declarations and the new harness probe.

## Parity evidence for conversation history/recovery slice (#1221)

`cargo fmt --all --check` clean; `cargo clippy -p quecto-tui --all-targets -D warnings` clean.

| Class | Surface | Evidence | Verdict |
|---|---|---|---|
| Behavioural | All ten contract surfaces | Whole workspace green after the extraction: 5052 tests, 192/192 BDD scenarios, 37/37 architecture tests, 31/31 contract tests. The 15 characterization tests and 11 `RangeAccumulator` tests written BEFORE the move passed unchanged after it. | PASS |
| Behavioural | Pre-existing test tree | `git diff b0584166..HEAD` over pre-existing test files shows changes in **five**, in three distinct classes. (1) MECHANICAL: `app_paged_history_review_tests.rs` and `app_paged_history_tests.rs` — every hunk a field-path rename (`session.history_before_cursor` → `session.history.before_cursor`) plus the `PendingHistoryPage` import path; zero assertions, fixtures, or test names changed. (2) DELIBERATE BDD REWRITES: `tui_paged_history_steps.rs` and `tui_paged_history_1094_steps.rs` — two step bodies rewritten and four orphaned step definitions deleted in response to review findings; these are behavioural test changes, NOT renames. (3) LOAD-BEARING GATE CHANGE: `quecto-agentic-harness/tests/architecture.rs` — `TUI_INTERFACE_RAW_JSON_SITE_SEED` lowered 130 → 120, a ratchet whose own doc comment says "Never raise it". This row has now been corrected twice: it first claimed "exactly two" files and "zero test names changed" (true only of class 1), then "four" (omitting class 3). Recorded here because a scope claim that keeps understating itself is worse than no claim. | PASS (all three classes disclosed) |
| Behavioural | Adapted tests still load-bearing | Re-ran mutation evidence after adapting them: correlating by id prefix instead of exact match fails `page_response_with_prefix_matching_id_is_rejected`; removing the staleness window fails both `stale_in_flight_page_is_retried_after_age_window` and `late_twin_of_stale_retried_page_is_dropped`. | PASS |
| Visual | Rendered frames | Every characterization and BDD assertion in this slice reads rendered frame text (`chat_text`, `active_chat_text`) or the app's own transcript entries. All pass unchanged, so no pinned frame differs. | PASS |
| Performance | Older-page emission | Old code inlined the guards in `next_history_page_request`; new code calls `HistoryPaging::next_page_request`. Same work: one `format!` and one cursor clone per REQUEST. The path is still scroll-driven — the only callers are `Key::ScrollUp`/`Key::PageUp` in `app_event_loop.rs` — so nothing was moved into the render loop. | PASS |
| Performance | Page correlation | Was an `Option<&str>` equality; still an `Option<&str>` equality inside `is_pending_page`. No allocation, no map lookup added. | PASS |
| Performance | Backfill reconcile | Still one `Vec<ChatEntry>` per payload and one prepend/replace pass. The policy returns a `PrefixPlan` enum (a `Copy`, allocation-free value) and the interface performs the single chat mutation; previously-loaded entries are still never recomputed. | PASS |
| Performance | Recovery trigger | `TurnOutcome` borrows `refs` and `assistant_text` (`&'a [String]`, `&'a str`) — it is a view, not a copy, so the per-turn heuristic allocates nothing. Previously the same fields were read directly off `App`. | PASS |
| Performance | Batch completion | `is_complete()` is the same `len == len` comparison. `ordered_by_refs` is a lazy iterator that now BACKS the ref-ordered walk `recovered_chat_entries` previously hand-rolled, so ordering has one implementation rather than two; the walk itself is unchanged (same skip on an absent ref, same single pass). Still one `replace_range` per batch, never incremental splices. | PASS |
| Quantitative | Trigger-logic duplication | The force-recovery check (`open_tool_calls > 0`) existed at 2 call sites (master + sub-agent); it now exists at 1, inside the policy. Both paths route through `TurnOutcome::needs_recovery`. | PASS |
| Quantitative | Production LOC | Conversation production code: 828 lines before (`app_paged_history` 316 + `app_message_recovery` 440 + `range_accumulator` 72) → 1060 after, measured by `wc -l` at the current head (`app_paged_history` 259 + `app_message_recovery` 423 + `range_accumulator` 76 + `history_paging` 187 + `turn_recovery` 115). Net +232. (An earlier version of this row read 1035/+207; those figures predated the `forced_without_text` and `ordered_by_refs` review fixes and were no longer reproducible.) The issue sets no LOC target; the growth is doc comments explaining the invariants (why exact-match correlation, why an open tool call forces recovery) that were previously implicit. Interface files themselves shrank by 74 lines (316+440 = 756 → 259+423 = 682), remeasured at head; the earlier "81" was never re-measured after the review fixes. | RECORDED |
| Quantitative | Testability criterion | 23 new tests construct the policy with no terminal, concrete client, raw JSON, or Tokio runtime. `grep` for `serde_json|ratatui|tokio|crossterm|Client` across `src/domain/*.rs` production files returns nothing. | PASS |
| Structural | `ChatEntry` stays a view projection | `grep -rn ChatEntry src/domain/ src/application/` returns nothing: policy vocabulary is `PageFacts`, `PrefixPlan`, `TurnOutcome`, `RecoveryBatch<T>`. | PASS |

## Review-round fixes for conversation history/recovery slice (#1221)

Seven narrow finders reviewed the PR in parallel; every consequential finding
was then dispatched to an adversarial verifier prompted to REFUTE it. Two were
refuted and correctly not acted on: `HistoryPaging`'s `pub` fields (zero
production writers — production uses methods exclusively, and `App.history` is
private, so no external consumer can reach a live instance) and
`is_complete()`'s count-vs-per-ref comparison (byte-identical to the
pre-refactor check, and unreachable since `messageRefs` carries unique ids).

Five findings survived and are fixed:

| Severity | Finding | Fix | Mutation evidence |
|---|---|---|---|
| HIGH | Moving `range_accumulator.rs` out of `interface/` dropped its 10 raw-JSON sites out of `tui_raw_json_parsing_sites_do_not_grow`'s scan root (measured 130 → 120) while the seed stayed 130, silently allowing 10 new interface-layer sites. | Lowered the interface raw-JSON seed to 120, then added a separate application raw-JSON seed at 69 so the moved parser remains measured. | Adding 10 raw `serde_json` sites to `interface/app_effort.rs` PASSED before the fix and FAILS after it. |
| HIGH | `a_control_sequence_split_across_recall_pages_is_sanitized_after_reassembly` was a deletion detector, not a policy detector: a per-page sanitizer consumes the dangling `ESC[` via the unterminated-CSI branch, leaving no ESC byte, so the sole assertion passed on the policy it documented. | Assert the payload too (`ends_with("RED") && !contains("31m")`). | Sanitizing per page now FAILS the test (it passed before); deleting the post-reassembly sanitize still FAILS. Both mutations caught. |
| MED | `ordered_responses()` had zero production consumers; `recovered_chat_entries` hand-rolled the same ref-ordered walk, so the ordering invariant was pinned on code that never ships. | Extracted `ordered_by_refs` as the single ordered-walk primitive and routed `recovered_chat_entries` through it. `ordered_responses()` was then DELETED — the conformance sweep caught that it was still dead after the reroute, since production calls the free function, not the method. Ordering now has one implementation. | Sorting ids instead of walking ref order FAILS `recovered_chat_entries_handles_suppressed_calls_errors_and_unknown_roles` — a test exercising the PRODUCTION walk. |
| MED | The parity-evidence row claimed `ordered_responses()` replaced "an equivalent walk"; it replaced nothing. | Row corrected to state what actually changed. Also fixed two stale doc lines from the move ("Seeded at 130", the old `interface/` path). | n/a (documentation). |
| LOW | The refactor lost a `&&` short-circuit: `latest_assistant_text()` (which clones the whole assistant body) became unconditional at the master site. Only the master site regressed — the sub-agent site was already unconditional. | Added `TurnOutcome::forced_without_text`, so the text is materialised lazily and the force rule stays INSIDE the policy rather than being re-duplicated at the call site. | Dropping the empty-refs guard from the fast path FAILS `the_text_free_fast_path_never_disagrees_with_the_full_policy`. |

One finding was raised as HIGH and declined as out of scope: `app_response.rs`
correlates resume/rewind responses by FIXED literal ids (`resume-messages`,
`rewind-refresh`), which are broadcast to every connected client, so a second
TUI can have its transcript replaced by a resume it never issued. The verifier
confirmed the transport fan-out is real (`uds.rs:168` selects
`EventSink::Broadcast` in multi-client mode) — but also confirmed it is
PRE-EXISTING and identical at the parent commit. Fixing it would be a
behavioural change, which this zero-behaviour-change slice forbids. Filed as
follow-up rather than smuggled in here.

## Adversarial review round for conversation history/recovery slice (#1221)

Three adversarial finders attacked the merged slice from distinct angles
(correctness of the extracted state machines, integration seams and
user-visible behaviour, and safety-net integrity). Every actionable finding was
re-verified locally by applying the exact mutation the finder claimed survives,
before any fix was written.

Verdict across all three: **no behavioural regression introduced by the
extraction.** Both the correctness and integration finders independently
concluded the zero-behaviour-change claim holds; `reconcile` is a line-for-line
transcription of the old inline arithmetic, `range_accumulator` is byte-identical
modulo visibility, and the two recovery predicates are provably equivalent given
the callers' `refs.is_empty()` pre-check.

What did NOT hold was the safety net's coverage. Four mutations were confirmed
invisible to the ENTIRE `quecto-tui` lib suite (1627 tests at that commit; 1639 at head), and all four are now killed:

| Severity | Gap | Mutation that survived | Now killed by |
|---|---|---|---|
| HIGH | `reconcile`'s prefix × latch matrix had two untested corners; the `ReplacePrefix` arm's prefix arithmetic was unpinned. | `(ReplacePrefix(partial_len), partial_len + facts.page_len)` — double-counts the prefix, so the NEXT snapshot's `replace_history_prefix` eats live transcript entries below the backfill. | `a_partial_snapshot_over_a_partial_prefix_replaces_it_without_double_counting` and `an_own_older_page_that_closes_the_backfill_clears_the_partial_prefix` |
| MED | The empty-page early return was only ever exercised against a virgin struct, so "an empty page never latches the guard" was unproven for latched or partial state. | Clearing `backfilled`/`partial_prefix_len` before the `page_len == 0` return — an empty broadcast snapshot un-latches a completed backfill and re-triggers a full re-page. | `an_empty_page_does_not_disturb_an_already_latched_backfill` and `an_empty_page_does_not_disturb_an_in_progress_partial_prefix` |
| MED | The in-flight dedupe's `pending.before == before` conjunct was never falsified: no test held a pending page for one cursor while requesting another. | Deleting the conjunct — once the cursor advances, the next page is suppressed for a full 30s retry window and scroll-back appears wedged. | `a_pending_request_for_a_different_cursor_does_not_suppress_a_new_one` and `no_id_correlates_when_nothing_is_in_flight` |
| MED | `LengthMismatch` was pinned SHORT but never LONG, and a non-string `content` was unpinned. | `self.content.len() >= content_len` — an over-long body ships as `Complete`. Both callers treat an unflagged body as trustworthy user content. | `a_final_page_longer_than_advertised_is_a_length_mismatch` and `a_non_string_content_field_is_missing_content` |

One claimed gap was REFUTED on verification: flipping `hasMoreContent`'s
`unwrap_or(false)` to `true` already fails 9 tests, so that default is covered.
One proposed test also encoded a wrong assumption about the
`next_offset == contentLength` boundary. (The correction made at the time was
ITSELF wrong — see rounds 2 and 3 below; the boundary is a valid continuation.)

Two documentation claims were found overstated and are corrected above: the
pre-existing-test-tree row (four files changed in two classes, not "exactly
two", and the BDD rewrites are genuine behavioural test changes) and the
production-LOC row (1060/+232 measured at head, not the pre-review 1035/+207).

Remaining findings were all confirmed **pre-existing at `b0584166`** and
behavioural to fix, so they are filed as follow-ups rather than changed inside
this zero-behaviour-change slice.

### Round 2: attacking the round-1 fix

The round-1 fix commit was itself put through an adversarial review. It found
that two of the four gaps were not actually closed, and that one "fix" was
wrong. Both were reproduced locally before acting.

| Severity | Finding | Resolution |
|---|---|---|
| HIGH | The `reconcile` matrix was closed to 6/8, not 8/8. Both `extend_prefix=true` × `partial_prefix_len=None` corners were still untested, and they are production-reachable: an empty or filtered snapshot leaves `partial=None, has_more=true`, and the next own older page lands there. `unwrap_or(0)` → `unwrap_or(1)` in the extend arm left the whole lib suite green (1636 tests at that commit) while producing the SAME user-visible bug the round-1 HIGH was about. | Added `an_own_older_page_with_no_recorded_prefix_counts_only_its_own_length` and `..._that_closes_the_backfill`. The mutation now fails. Matrix is 8/8. |
| HIGH | `a_next_offset_exactly_at_the_advertised_end_is_invalid_progress` passed for the WRONG reason. Its fixture double-counted its own seed (`acc("abc", 0)` plus a 3-byte page against a 3-byte total = 6 accumulated bytes), so it died on the overshoot conjunct and never reached the boundary it was named for. Probing the real boundary returns `Ok(Continue { next_offset: 3 })` — the original expectation was correct, and the author had bent the test to match the implementation rather than investigating. | Test corrected to assert `Ok(Continue)` and renamed `..._is_a_valid_continuation`, with the fixture mistake recorded in its doc comment. A separate `accumulating_beyond_the_advertised_length_is_still_invalid_progress` was added for the overshoot case. NOTE: as first written that test was itself broken in the same way — see round 3 below. Tightening the guard to `>=` now fails. |
| MED | The corrected pre-existing-test-tree row was STILL wrong: five files changed, not four. `architecture.rs` — a lowered never-raise ratchet seed — is a third, load-bearing class. | Row corrected to three classes, with a note that it has now understated its own scope twice. |
| MED | The "remeasured by `wc -l`" LOC row still carried an unmeasured 81-line figure (actual: 74). | Remeasured and shown as arithmetic. |

The lesson recorded for the next slice: **bending a failing test's expectation
to match the implementation is not a fix.** When the round-1 boundary test
failed, the correct response was to probe what the code actually does at the
true boundary — which would have shown the original expectation was right.

Both round-2 HIGHs were mistakes in the round-1 FIX, not in the extraction
itself. Every adversarial angle continues to agree that the extraction
introduces no behavioural regression.

### Round 3: the same defect class, reproduced by the fix for it

Round 3 attacked the round-2 fix and found that it had reproduced the exact
defect it was written to eliminate.

| Severity | Finding | Resolution |
|---|---|---|
| HIGH | `accumulating_beyond_the_advertised_length_is_still_invalid_progress` never reached the overshoot guard. Its fixture used `offset: 3, nextOffset: 3` — carried over from the boundary case it was split away from — so `next_offset <= response_offset` (3 ≤ 3) short-circuited first. Deleting `\|\| self.content.len() > content_len` left the test named for overshoot GREEN; only the older `accumulating_past_the_advertised_length_is_invalid_progress` caught it. | Fixture changed to `acc("abcdef", 6)` + a 2-byte page against a 7-byte total, which satisfies every other conjunct so only overshoot can reject it. Verified: deleting the overshoot conjunct now fails BOTH overshoot tests. |
| HIGH | The round-2 table claimed the new test "covers the overshoot case the broken fixture was accidentally testing". False in both directions: the deleted fixture genuinely did pin overshoot, and the replacement did not. The change REMOVED a working pin while documenting that it added one. | Claim corrected. A reviewer trimming the older test as redundant would have dropped overshoot coverage to zero. |
| MED | Docs asserted a redundancy that did not exist, presenting a single point of failure as doubly covered. | Corrected; the guard-isolation table below now records which tests actually kill which mutation. |

**Discipline adopted, and the reason it was needed.** Three consecutive rounds
produced the same class of error: a negative test that passes for a reason
other than the one it is named for. The rule that catches all of them —
including round 2's HIGH and round 3's own first proposed fix, which merely
shifted the death to a third conjunct — is:

> Every negative test must be verified to FAIL when the specific guard it names
> is removed. Passing is not evidence; dying for the right reason is.

Guard-isolation sweep at head, each mutation applied in isolation:

| Mutation | Tests that die |
|---|---|
| `unwrap_or(0)` → `unwrap_or(1)` in the extend arm | `an_own_older_page_with_no_recorded_prefix_counts_only_its_own_length` |
| `ReplacePrefix` arm returns `partial_len + page_len` | `a_partial_snapshot_over_a_partial_prefix_replaces_it_without_double_counting` |
| drop the `trimmed` conjunct from the latch | 3 tests: `a_trimmed_or_incomplete_page_leaves_the_backfill_open`, `trimmed_busy_connect_snapshot_then_full_attach_backfill_restores_older_history`, `a_trimmed_page_advertising_more_history_stays_open_to_later_snapshots` |
| drop the `has_more_before` conjunct from the latch | 11 tests, incl. `a_later_snapshot_replaces_the_whole_loaded_prefix` and `older_history_page_prepends_without_gap_or_duplicate` |
| delete the overshoot conjunct | `accumulating_beyond_the_advertised_length_is_still_invalid_progress` AND `accumulating_past_the_advertised_length_is_invalid_progress` |
| `next_offset <= response_offset` → `false` | 1 test (isolated) |
| `next_offset > content_len` → `false` | 1 test (isolated) |

Round 3 also independently re-derived the boundary assertion rather than
reading it off the implementation, and confirmed `Ok(Continue { next_offset: 3 })`
is correct on its own merits: the state cannot loop, because
`next_offset <= response_offset` terminates it on the very next hop. It further
verified the interface raw-JSON seed `120` is exact — the `<=` comparison means a
green test alone proves nothing, so it lowered the seed to 119 and confirmed the
assertion reports `found 120`. For the first time in three rounds every
documented number checks out.

### Round 4: the discipline applied only where it was already looking

Round 4 swept EVERY negative-asserting test in the slice with the round-3
guard-removal rule, including the characterization suite that no earlier round
had checked this way. Verdict: round 3 did not break the streak — it made it
four.

| Severity | Finding | Resolution |
|---|---|---|
| HIGH | `a_trimmed_page_advertising_more_history_stays_open_to_later_snapshots` survived removal of the `facts.trimmed` guard it is named for. Its fixture set BOTH `trimmed: true` and `hasMoreBefore: true`, so the second sufficient condition held the latch open and the test stayed green. It died only under an unconditional latch. | Fixture changed to `trimmed: true, hasMoreBefore: false`, so only `trimmed` can keep the backfill open. Verified: dropping the `trimmed` conjunct now fails this test. |
| MED | The guard-isolation table — created in round 3 as the durable remedy — was wrong in 2 of its 7 rows, both understating coverage: `trimmed` listed 1 killer (actual 3), `has_more_before` listed 4 (actual 11). | Both rows remeasured against the real failure sets. |
| MED | A round-1 sentence still asserted the `next_offset == contentLength` boundary is `InvalidProgress`, contradicting round 2's correction and the shipped code. | Corrected, and annotated to record that the round-1 "correction" was itself wrong. |
| LOW | Absolute suite counts (1627, 1636) were unverifiable at head. | Rewritten to name the commit they were measured at and the count at head (1639). |

Round 4 also confirmed the three pure-policy files are genuinely clean: all 24
isolated guard-removal mutations across `range_accumulator_tests.rs`,
`history_paging_tests.rs` and `turn_recovery_tests.rs` killed their named test
and no other. The corrected overshoot fixture is right this time, its sibling is
not redundant, the interface raw-JSON seed `120` is exact, and every LOC figure
reproduces by `wc -l`.

**What four rounds actually demonstrated.** The recurring failure was never the
extraction — every round independently confirmed it introduces no behavioural
regression. It was that each round adopted a correct rule and then applied it
only to the files already in front of it. Round 3 wrote the guard-removal rule
and left the same defect live one directory away, and encoded two wrong counts
in the very table meant to prevent that. A rule is not a remedy until it is
applied exhaustively to every test the claim covers, and the artefact recording
it is itself measured rather than asserted.

### Round 5: the prose remedy replaced with an executable one

Round 5 swept 44 negative-asserting tests and inverted 7 positive ones across
all five test files, running 55 isolated single-guard mutations. It found that
round 4 had again stopped one function short.

| Severity | Finding | Resolution |
|---|---|---|
| HIGH | `HistoryPaging::reset` and `reopen_backfill` had NEVER been mutated by any round. Three of their guards were pinned only by vacuous assertions: the fixtures left each field already in its post-condition state before the method ran. Deleting `reset`'s `backfilled = false` or `reopen_backfill`'s `partial_prefix_len = None` killed ZERO tests workspace-wide. Production impact: a resume/rewind/clear leaves the latch set and every future attach-backfill is suppressed; a stale prefix length feeds the next `ReplacePrefix` and deletes live transcript entries — the round-1 HIGH's exact bug class. | Both tests rewritten so every cleared field is non-default beforehand, plus `reset_clears_a_latched_guard_that_has_no_partial_prefix`. All seven lifecycle guards now die when removed. |
| HIGH | The round-4 doc claim "all 24 isolated guard-removal mutations killed their named test and no other" was unfalsifiable — it enumerated no mutation, and "and no other" was contradicted by the guard-isolation table two paragraphs above it. | Claim deleted and replaced by `scripts/check-guard-manifest.sh`. |

**The remedy is now executable, not prose.** `scripts/check-guard-manifest.sh`
enumerates the guards in the history paging, turn recovery, and range assembly
policy files; deleting each listed guard must fail at least one test. Because the
manifest performs many full lib-test mutations, the pre-push gate exposes it as
an opt-in lane (`QUECTO_RUN_GUARD_MANIFEST=1`) rather than running it on every
push; that opt-in lane has its own cache key, so a cached default push cannot
silently skip an explicitly requested manifest run. Running it immediately found
a **ninth** unpinned guard that five rounds of manual review had missed —
`reconcile`'s keep-open arm never observably unlatched `backfilled`, because no
test fed a partial page to an already-latched backfill. Fixed by
`a_partial_page_unlatches_a_previously_completed_backfill`.

Writing the manifest also exposed two bugs in the manifest itself, both of the
same family the reviews kept finding: it reported a compile failure as a
surviving guard (a `sed` line-deletion inside a multi-line boolean does not
parse), and it matched cargo's `error: test failed` summary — which appears on
every legitimate kill — as a build failure. Both now distinguished explicitly,
with a `check_replace` form for multi-line guards.

**Conclusion after five rounds.** Every round independently confirmed the
extraction introduces no behavioural regression; all findings were in the
safety net and the documentation, never in the shipped refactor. The recurring
failure was that each round applied a correct rule to exactly the files the
previous round's findings pointed at. The scoping was the defect, not the
rigour — which is why the remedy had to become a checked-in enumeration that
cannot silently omit a function. An unlisted guard is now an unverified guard.
