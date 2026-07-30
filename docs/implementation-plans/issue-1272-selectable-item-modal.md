## Implementation Plan for #1272 — Reusable generic modal for selectable item lists

### Grounding and verified starting state

Goal: add a reusable TUI modal/list-management component for collections of enabled/disabled items. It must support generic item data, stable IDs and labels, optional description/metadata, fuzzy filtering, individual toggles, explicit bulk enable/disable semantics, clear cancel/apply semantics, and tests for filtering, bulk actions, toggles, and cancel/apply.

Constraints and conventions verified in the current code:
- The likely home is `quecto-tui/src/components`, which exposes shared UI components through `quecto-tui/src/components/mod.rs`.
- Existing modal composition is split between list state/rendering and overlay framing: `SelectList` owns items/navigation/result and implements `Component` (`quecto-tui/src/components/select_list.rs:22-28`, `quecto-tui/src/components/select_list.rs:84-125`), while `select_overlay::build_select_overlay` wraps arbitrary rendered content in the shared modal frame (`quecto-tui/src/components/select_overlay.rs:68-120`).
- `SelectList` currently supports stable `value`, label, optional description (`quecto-tui/src/components/select_list.rs:9-15`), Up/Down/Enter/Escape (`quecto-tui/src/components/select_list.rs:110-123`), windowed rendering (`quecto-tui/src/components/select_list.rs:93-106`), and empty state (`quecto-tui/src/components/select_list.rs:88-90`), but it does not support filtering, toggle state, or apply/cancel state snapshots.
- Existing selector surfaces (`ModelSelector`, `EffortSelector`) independently implement fuzzy query state and render search lines (`quecto-tui/src/components/model_selector.rs:94-97`, `quecto-tui/src/components/model_selector.rs:234-240`, `quecto-tui/src/components/effort_selector.rs:30-35`, `quecto-tui/src/components/effort_selector.rs:103-108`), which is evidence that this new component should reuse existing fuzzy/list infrastructure and avoid duplicating future selector behavior.
- Fuzzy matching is already implemented in `quecto-tui/src/components/fuzzy.rs`; `fuzzy_filter` supports space-separated token matching and stable best-first ordering (`quecto-tui/src/components/fuzzy.rs:87-95`, `quecto-tui/src/components/fuzzy.rs:126-172`).
- Shared navigation and windowing exist in `ListNavigator` (`quecto-tui/src/components/list_navigator.rs:14-73`) and `list_rows::render_windowed` (`quecto-tui/src/components/list_rows.rs:54-128`); the new component should use them rather than inventing navigation/windowing.
- `quecto-tui` is versioned in `quecto-tui/Cargo.toml:1-4`. The workspace harness also pins a path dependency version for `quecto-tui` in `quecto-agentic-harness/Cargo.toml`; implementation must grep and update any version pins in lockstep.

### Acceptance criteria checklist

- [ ] A reusable modal/list-management component exists in or near `quecto-tui/src/components` and is discoverable by module name/docs.
- [ ] The component accepts generic item data via stable ID, label, optional description, and optional search/metadata accessors.
- [ ] Fuzzy search filters across labels and useful metadata.
- [ ] Users can toggle individual items on/off.
- [ ] Users can enable all visible items and disable all visible items.
- [ ] Any all-items bulk operations use explicit method/action names; visible-vs-all semantics are never ambiguous.
- [ ] Search can be cleared and empty results are represented predictably.
- [ ] Apply returns the working enabled set; dismiss/cancel returns a dismissed result and does not commit the working set, using the existing `Pending`/terminal-result/`Dismissed` pattern rather than a separate `Cancelled` vocabulary.
- [ ] Keyboard interactions are practical and consistent with the TUI (`Up`/`Down`, printable text input, `Backspace`, `Escape`, `Enter`; plus explicit non-`Char` toggle/bulk shortcuts documented in footer/render text).
- [ ] Tests cover filtering, bulk enable/disable, individual toggles, and cancel/apply behavior.

## Execution Checklist

- [ ] Phase 1 — Implement the reusable modal component behind focused tests
- [ ] Phase 2 — Prove reusable API shape and protect existing list behavior
- [ ] Phase 3 — Polish verification, versioning, and handoff checklist

## Phase 1 — Implement the reusable modal component behind focused tests

Deliverable: a green, reusable `quecto-tui` selectable-item modal component with unit/render tests for the issue's core behavior. This phase is independently shippable: it adds the reusable component and its modal framing, but does not wire production model/tool/workflow management screens.

Files touched:
- `quecto-tui/src/components/selectable_item_modal.rs` (new component, module docs, component tests via `#[path = "selectable_item_modal_tests.rs"]`).
- `quecto-tui/src/components/selectable_item_modal_tests.rs` (new focused unit/render tests).
- `quecto-tui/src/components/mod.rs` (add `pub mod selectable_item_modal;`).
- Optionally `quecto-tui/src/components/select_overlay.rs` only if the modal-frame helper should live beside existing overlay helpers instead of inside the new module.

Proposed API shape:
- Convert generic caller data into owned internal rows at construction to avoid lifetime-heavy UI state. A builder/config should accept `items`, `get_id`, `get_label`, optional `get_description`, and optional `get_search_text`/metadata hook.
- `SelectableItemModal` (or similarly named type) should implement `Component: Send`, so it renders with `render(&mut self, width) -> Vec<String>`, consumes keys through `handle_input(&mut self, &Key) -> bool`, and keeps every rendered line within the requested visible width.
- Follow existing result conventions: define `SelectableItemModalResult` (or `SelectableItemResult`) with `Applied(BTreeSet<String>)`, `Dismissed`, and `Pending`; expose `take_result(&mut self)` that resets to `Pending`. Avoid introducing a separate `Cancelled` result name because existing list/select surfaces use `Dismissed`.
- `SelectableItemModal` keeps:
  - all rows in stable order;
  - filtered visible row IDs/indices;
  - `original_enabled: BTreeSet<String>` unless implementation evidence favors another deterministic set type;
  - `working_enabled: BTreeSet<String>` for stable apply results;
  - bounded `query` using a module-level `MAX_QUERY_LEN` constant;
  - `ListNavigator` or `SuggestionList`-style state for visible selection/windowing;
  - cached visible label width if using `DescriptionMode::AlignedCached`, recomputed only when the filter changes rather than during every render.
- Public methods should make semantics unambiguous and follow existing naming/accessor style: `take_result`, `selected_item`, `visible_count`, `toggle_selected`, `enable_visible`, `disable_visible`, and only add `enable_all_items`/`disable_all_items` if those all-item operations are needed and explicitly named.
- Add a modal helper such as `build_selectable_item_modal_overlay(title, footer, modal, terminal_width, terminal_height)` that delegates to the existing `build_select_overlay` closure API; keep it `pub(crate)` unless a real external crate boundary requires `pub`.

Provider/consumer API shape:
- Introduce a small adapter trait or builder-facing provider contract so callers expose domain items without coupling the modal to models/tools/workflows. The modal should own its normalized rows after construction, while providers remain responsible for loading domain data and persisting the applied enabled IDs.
- Recommended contract shape, naming to be finalized during implementation:

  ```rust
  pub(crate) trait SelectableItemProvider {
      type Item;

      fn items(&self) -> Vec<Self::Item>;
      fn enabled_ids(&self) -> BTreeSet<String>;
      fn id(&self, item: &Self::Item) -> String;
      fn label(&self, item: &Self::Item) -> String;
      fn description(&self, item: &Self::Item) -> Option<String> {
          None
      }
      fn search_metadata(&self, item: &Self::Item) -> Vec<String> {
          Vec::new()
      }
      fn apply_enabled_ids(&mut self, enabled_ids: BTreeSet<String>);
      fn dismiss(&mut self) {}
  }
  ```

- Also support closure/builder construction for lightweight call sites that do not need a named provider type:

  ```rust
  let modal = SelectableItemModal::builder()
      .items(provider.items())
      .enabled_ids(provider.enabled_ids())
      .id(|item| item.id.clone())
      .label(|item| item.display_name.clone())
      .description(|item| item.summary.clone())
      .search_metadata(|item| vec![item.kind.clone(), item.provider.clone()])
      .build()?;
  ```

- Provider integration should be explicit and two-phase:
  - providers pass the original enabled set into the modal;
  - the modal mutates only its working set while open;
  - callers inspect `take_result()`;
  - callers invoke `provider.apply_enabled_ids(ids)` only for `Applied(ids)`;
  - callers invoke `provider.dismiss()` or no-op cleanup for `Dismissed`.
- Provider-owned domain item types should never leak into the modal's public result. Results should be stable IDs only, keeping persistence and side effects in the provider layer.
- Provider tests should include model/tool/workflow-shaped fixtures that implement the same provider contract and assert that each can construct the modal without duplicating filter/toggle/apply logic.
- Keep provider contract visibility `pub(crate)` unless a concrete external crate needs to consume it. If only tests need polymorphism, prefer the builder API plus fixture helpers over a trait that would be unused by production code.

### Task checklist

- [ ] Add focused tests first for fuzzy filtering across label + metadata, empty state after no matches, clear-search behavior, per-item toggle, visible bulk enable/disable, apply result, dismissed result, and `take_result` resetting to `Pending`.
- [ ] Use render tests to prove title/search/footer/empty state/enabled markers appear inside the shared modal frame and every rendered line obeys `visible_width(line) <= width`.
- [ ] Reuse `fuzzy_filter` or `fuzzy_match` for filtering; search text should include label plus description/metadata.
- [ ] Reuse `ListNavigator` and `list_rows::render_windowed` (or `SuggestionList` if its active/selection semantics fit without bending it) for navigation/rendering.
- [ ] Render stable enabled/disabled markers, label, and optional description using existing theme/list row utilities; avoid raw width math where `ListRow`/`truncate_to_width` already cover it.
- [ ] Sanitize control characters in generic labels/descriptions/metadata before storing/rendering/searching, following the existing `ansi::sanitize_control` terminal-injection safeguard.
- [ ] Pin the keymap in tests and footer text before GREEN. Recommended keymap: `Space` toggles selected, `CtrlShift('a')` enables visible, `CtrlShift('d')` disables visible, `Enter` applies, `Esc` dismisses, printable text input filters, and `Backspace` edits search. Avoid `Ctrl+D` because the app treats it as unconditional exit before overlays, avoid `Ctrl+A` because it is editor home, and avoid bare `A`/`D` because `Key::Char` is search input.
- [ ] Preserve existing overlay behavior: when this modal is wired later, overlay-specific routing must happen before editor/autocomplete handling, and `handle_input` should return `false` for unrelated keys.
- [ ] Clamp selection predictably when filtering changes, matching existing selector behavior; do not reset to row 0 merely because the query changed.
- [ ] Keep all mutation in the working set until apply; dismiss must not expose changed state as committed.
- [ ] Enforce and test a bounded max query length with `query.len() < MAX_QUERY_LEN` style behavior matching existing selectors.
- [ ] Avoid consumer-specific model/tool/workflow assumptions.

Verification:
- RED evidence: targeted new tests fail before implementation.
- GREEN evidence: `cargo test -p quecto-tui --lib selectable_item_modal` passes after implementation.

## Phase 2 — Prove reusable API shape and protect existing list behavior

Deliverable: confidence that the component can serve models, tools, and workflows without duplicating core modal logic, while not regressing existing selector/list behavior.

Files touched:
- `quecto-tui/src/components/selectable_item_modal_tests.rs`.
- Existing characterization tests only if they need a small addition to cover a regression risk; avoid changing production selector consumers in this phase.

### Task checklist

- [ ] Add compile-time/unit fixtures using at least three different item structs or row sources representing models, tools, and workflows; each should construct the same component through accessors without duplicating filter/toggle/apply logic.
- [ ] Add tests proving visible bulk actions affect only the filtered visible subset and leave hidden items unchanged.
- [ ] Add tests proving optional all-item methods, if implemented, affect all items and have distinct names from visible actions.
- [ ] Add tests for duplicate-ID rejection or deterministic duplicate handling; prefer rejecting duplicates at construction because the issue requires stable unique IDs and `selected_item`/apply results should not be ambiguous.
- [ ] Add tests for selection behavior when filtering removes the selected row, preserving current TUI convention of clamping rather than panicking or resetting unnecessarily.
- [ ] Add tests for `handle_input` consumed/unconsumed return values, matching `Component` expectations.
- [ ] Run existing targeted list/selector tests that cover shared infrastructure likely to be affected: `select_list`, `select_overlay`, `list_rows`, `suggestion_list`, `autocomplete`, `model_selector`, and `effort_selector`.
- [ ] Do not add BDD feature files for this no-consumer slice unless the feature workflow requires them; if required, add/verify corresponding step definitions and harness integration, keep scenarios declarative, and describe reusable modal behavior rather than key presses or Rust APIs.

Verification:
- `cargo test -p quecto-tui --lib selectable_item_modal`
- `cargo test -p quecto-tui --lib select_list`
- `cargo test -p quecto-tui --lib select_overlay`
- `cargo test -p quecto-tui --lib list_rows`
- `cargo test -p quecto-tui --lib suggestion_list`
- `cargo test -p quecto-tui --lib autocomplete`
- `cargo test -p quecto-tui --lib model_selector`
- `cargo test -p quecto-tui --lib effort_selector`

## Phase 3 — Polish verification, versioning, and handoff checklist

Deliverable: implementation-ready final checklist for the feature workflow's verify/version/PR stages.

Files touched:
- `quecto-tui/Cargo.toml` patch or minor bump as required by repo policy.
- Any path-dependency/version pin found by grep, including `quecto-agentic-harness/Cargo.toml` if its `quecto-tui` dependency version must stay aligned.
- Any TUI version assertion/docs if grep finds one.

### Task checklist

- [ ] Run `cargo fmt`.
- [ ] Run targeted strict clippy for the changed package: `cargo clippy -p quecto-tui --all-targets -- -D warnings`.
- [ ] Re-run targeted component and regression tests listed above.
- [ ] Bump `quecto-tui` version because TUI source changed; update lockstep version pins/docs/tests discovered by grep.
- [ ] In the feature workflow, stage only intended files; commit on a feature branch; push through pre-push hooks; open PR.
- [ ] In the feature workflow, complete required BDD review (if BDD scenarios were added), PR review, conformance, and authoritative CI steps before handoff.

Verification:
- Formatting, clippy, targeted tests, version grep, pre-push gate, PR review/conformance, and authoritative CI all pass during implementation.

## Non-goals

- Do not wire production model/tool/workflow management screens in this issue unless a current caller already exists and integration is trivial; the proof of reuse should be tests/fixtures, not three production surfaces.
- Do not introduce a web UI dependency or external fuzzy-search crate; reuse the existing TUI fuzzy implementation.
- Do not support disabled/unavailable items as a separate first-class state unless a concrete consumer requirement emerges; model it later as optional row metadata or a follow-up issue.
- Do not silently make bulk actions operate on both all items and visible items with the same label; semantics must be explicit.

## Review resolutions

- Sequencing review: accepted. The original RED-only phase was not independently green; the plan now combines tests, implementation, and modal helper in Phase 1 so the first implementation phase is shippable.
- Sequencing review: accepted. Standalone BDD feature work is now optional/deferred for this no-consumer component slice; focused unit/render tests are primary, with BDD only if required by the implementation workflow.
- Sequencing review: accepted. The modal helper moved into Phase 1 so the deliverable is a reusable modal, not just a list manager.
- Sequencing review: accepted. PR/process work is separated into a handoff checklist rather than treated as an implementation dependency; it is retained as feature-workflow guidance, not issue acceptance criteria.
- Citation review: accepted. The plan now cites concrete file:line evidence, calls out lockstep `quecto-tui` dependency version pins, and labels visible-bulk behavior as an explicit design decision.
- Regression/API-standard review: accepted. The plan now requires `Component`/`take_result`/`Pending`-reset conventions, width-bounded rendering, control-character sanitization, non-`Char` bulk shortcuts that avoid app-global `Ctrl+D` and editor `Ctrl+A`, clamping semantics, max-query-length coverage, duplicate-ID handling, and regression tests for existing list/selector/autocomplete surfaces.

## Open decisions for the implementer

- Use `Space` for selected-item toggle, `CtrlShift('a')`/`CtrlShift('d')` for visible bulk actions, and `Enter` for apply unless implementation review finds an existing TUI convention that strongly prefers another mapping; whatever mapping is chosen must be pinned in tests and footer text before GREEN.
- Visible bulk actions are the primary rendered actions for this slice. This is a design decision because issue 1272 does not specify visible-vs-all semantics; all-item operations may exist only with explicit names and separate tests.
