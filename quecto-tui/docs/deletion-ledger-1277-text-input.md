# Deletion ledger — #1277 text-input system

## Production moves

| Deleted / moved | Invariant | Re-established where |
|---|---|---|
| `components/editor.rs` monolith (~584 lines) | Multi-line draft, cursor, submit, history, render | `components/text_input/editor.rs` + `history.rs` |
| Inline `history: Vec`, `history_index`, `saved_text` on Editor | Cap 500, dedupe last, draft save/restore on Up/Down | `text_input::history::InputHistory` |
| `MAX_HISTORY` const on Editor | Cap at 500 | `history::MAX_HISTORY` |
| Public field access path for internals | External mutation only via API | All Editor fields private |

## Compatibility

| Item | Rationale |
|---|---|
| `components/editor.rs` re-exports `text_input::Editor` | Existing paths keep compiling; single implementation |

## Test adaptations (mechanical, logged)

Frozen characterization moved to `text_input/editor_tests.rs`. Field-poke assertions
replaced with observable behaviour (same behaviours, no production cfg(test) forks):

| Old assertion | New pin | Behaviour preserved |
|---|---|---|
| `e.lines.len() == 2` after Shift+Enter | Up + insert within multi-line draft | Multi-line buffer exists |
| `e.cached_lines.is_some/none` | render equality before/after invalidate | Cache hit + invalidate |
| `e.lines.len() == 1` after set_text("") | type `x` → `"x"` | Single empty line after clear |
| `use super::*` from old editor module | Explicit imports + Component/Key | Compile under new module tree |
| Direct private `navigate_history_*` / `word_*` / boundary helpers | `pub(super)` within text_input only | Same tests; not crate-public |

## Declined consolidations

| Pattern | Rationale |
|---|---|
| Absorb slash/files autocomplete into text_input | Issue scope: main prompt field only |
| Persist history to disk | Out of scope; in-memory only today |
