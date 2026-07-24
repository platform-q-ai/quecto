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
| `application/session_payloads.rs` | typed parsing foothold for session payloads | `protocol` mapping feeding `sessions` |
| `domain/` | placeholder for invariant-bearing shared values | only keep values that encode real invariants |

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
| Frozen characterization suite | Frozen files were not edited after the freeze point. | `git hash-object` values still match the freeze manifest: `architecture.rs` `70f2d8b3635574da3a5f42406ac6726a072deb3b`; `tui_architecture_steps.rs` `6b1b263cc161f552f856cfeb681150d8b3d2e757`; `tui_clean_architecture.feature` `b331f192c2be90bc567932f82efab696c0637b44`. | PASS |
| Visual parity | No render frames or visual surfaces are touched. | No files under `quecto-tui/src` changed; this slice only changes docs, tests, Cargo metadata, and lockfile. | PASS |
| Performance parity | No specialized runtime/rendering code is replaced, so no allocation, cache, single-pass, or complexity characteristic changes. | No production code changed. Targeted clippy passed: `cargo clippy -p quecto-agentic-harness --test architecture -- -D warnings` and `cargo clippy -p quecto-tui --all-targets -- -D warnings`. | PASS |
| Quantitative criteria | The issue asks for a net LOC decrease over the broader refactor epic; this slice deliberately adds architecture contract documentation and guardrails. | `git diff --shortstat HEAD~1..HEAD`: 8 files changed, 300 insertions(+), 15 deletions(-). Recorded as a review-time epic metric, not a test assertion. | PASS for slice / epic metric pending later code-moving slices |

Additional clean checks for this parity pass:

- `cargo fmt --all -- --check`
- `cargo clippy -p quecto-agentic-harness --test architecture -- -D warnings`
- `cargo clippy -p quecto-tui --all-targets -- -D warnings`
- `cargo test -p quecto-agentic-harness --test architecture tui_feature_oriented_architecture_is_documented`
- `QUECTO_TAG=issue-1149 cargo test -p quecto-agentic-harness --features test-support --test bdd`
- `cargo test -p quecto-agentic-harness --test architecture`
- `cargo test -p quecto-agentic-harness --features test-support --test bdd --no-run`
- `cargo check -p quecto-tui`

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
