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
| `interface/app_ledger_sync.rs` | `agents` |
| `interface/app_message_recovery.rs` | `conversation` |
| `interface/app_methods.rs` | `shell` composition methods until split by feature |
| `interface/app_models.rs` | `inference` |
| `interface/app_paged_history.rs` | `conversation` |
| `interface/app_response.rs` | `conversation` |
| `interface/app_resumed_history.rs` | `conversation` |
| `interface/app_rewind.rs` | `conversation` |
| `interface/app_selection.rs` | `shell` focus/routing until delegated to feature views |
| `interface/app_stdin.rs` | `conversation` input coordination with `shell` stdin adapter |
| `interface/app_subagent_feed.rs` | `agents` |
| `interface/app_subagent_panel.rs` | `agents` |
| `interface/app_subagent_state.rs` | `agents` |
| `interface/app_subagent_stream.rs` | `agents` |
| `interface/app_subagents.rs` | `agents` |
| `interface/app_submit.rs` | `conversation` |
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
| `interface/feed_state.rs` | `agents` pure/presentation state |
| `interface/fuzzy.rs` | `components` helper used by overlays/autocomplete |
| `interface/keys.rs` | `shell` input mapping primitive |
| `interface/kitty.rs` | `shell` terminal integration |
| `interface/ledger_sync.rs` | `agents` pure/presentation state |
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

- `tui_raw_json_parsing_sites_do_not_grow` — seed `130`.
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

Seeded at 130 sites. The ratchet's failure message reprints this inventory in
burn-down order, so it stays accurate without manual upkeep:

| Module | Sites |
|---|---|
| `interface/components/workflow_bar.rs` | 38 |
| `interface/app_events.rs` | 20 |
| `interface/app_subagent_stream.rs` | 19 |
| `interface/components/chat_render.rs` | 17 |
| `interface/range_accumulator.rs` | 10 |
| `interface/app_paged_history.rs` | 6 |
| `interface/ledger_sync.rs` | 6 |
| `interface/app_rewind.rs` | 5 |
| `interface/components/footer.rs` | 4 |
| `interface/app_subagents.rs` | 3 |
| `interface/app_effort.rs` | 2 |

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
