# Parity evidence — #1277 text-input system

## Fmt / clippy

| Check | Result |
|---|---|
| `cargo fmt -p quecto-tui` | clean |
| `cargo clippy -p quecto-tui --all-targets -- -D warnings` | clean |

## (1) Behavioural

Frozen characterization + supporting suites GREEN after refactor:

| Suite | Result |
|---|---|
| `components::text_input` (62 tests) | ok |
| `app_event_loop_tests` (60) | ok |
| `app_input_paste` (8) | ok |
| `app_rewind_response` (37) | ok |
| `default_invalidate` (2) | ok |

### Test-tree delta vs freeze

| Path | Delta | Rationale |
|---|---|---|
| `components/editor_tests.rs` | **moved** → `text_input/editor_tests.rs` | Mechanical module relocation with the system |
| Field-poke asserts (`lines.len`, `cached_lines`) | Observable equivalents | Sealed fields; same behaviours (deletion ledger) |
| Imports | `Component`/`Key`/`visible_width` explicit | Module path change |
| `app_event_loop_tests.rs` | +3 tests (pre-freeze) | Characterization gaps; frozen before production edit |
| Production `cfg(test)` forks | **none** | `render_line_with_cursor` re-export is `#[cfg(test)]` wrapper only (no behaviour fork) |

Adapted asserts re-mutation-tested: multiline Up→history mutation fails `multiline_input` (`"a\nbX"` vs `"aX\nb"`).

## (2) Visual

Pinned render behaviours still asserted:

- Borders + `>` / `!` indicators (`render_has_borders`, bash/normal mode tests)
- Width clamp (`render_respects_width`)
- Reverse-video cursor + hide (`hidden_cursor_emits_no_reverse_video`)
- Mid-char defensive render helper tests
- Cache reuse equality (`render_cache_reused`, `invalidate_clears_cache`)

No production render path changed except history field storage (not painted).

## (3) Performance

| Old specialized path | Characteristic | New path |
|---|---|---|
| `Vec` history + `remove(0)` over 500 | Same O(n) rare eviction | `InputHistory::push` identical |
| History nav: index + `set_text` clone | O(1) index + one draft replace | Unchanged mechanism |
| Render cache by width | Skip rebuild on hit | Unchanged fields/logic |
| In-place line `String` insert/delete | No full-buffer rebuild | Unchanged |
| Paste single-pass chars | Same | Unchanged |

No new allocations on the keystroke hot path; history is a pure move of the same `Vec`/`isize`/`String` trio into a private type.

## (4) Quantitative

| Metric | Value |
|---|---|
| Production text-input LOC (`editor`+`history`+`mod`+compat re-export) | **735** lines |
| Pre-refactor production `editor.rs` | **584** lines (history inlined) |
| Net structure | History extracted (+~128 dedicated); compat shim 7 lines; API docs in mod |
| Issue claim | Structural extraction (not a net-LOC-decrease mandate) — **met** via dedicated system module |
| External field mutation | **0** (all fields private) |
| Parallel history stores | **0** (only `InputHistory` inside Editor) |

## Structural goals (review-time)

| Goal | Evidence |
|---|---|
| Named text-input system | `components/text_input/` with module API docs |
| History owned by system | `history.rs` + `Editor::add_to_history` only |
| External access minimized | Private fields; public methods only |
| API documented | `text_input/mod.rs` boundary table |
| Call sites use API | `shell/app.rs` imports `text_input::Editor`; event loop uses methods only |
