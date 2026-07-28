# Parity contract — #1277 TUI text input system

**Status: APPROVED** (readiness gate passed; no `__UNRESOLVED__` markers)

**Issue:** #1277 — Refactor TUI text input into dedicated system with history management.

**Classification:** zero behaviour change (structural extraction/centralization only). Discriminator: issue acceptance criteria are structural/parity-only ("Existing text input behavior is preserved"; "Direct access/mutation … removed or minimized to rendering-only accessors").

## Readiness gate

| Check | Result |
|---|---|
| Issue mandates zero behaviour change | PASS — AC: "Existing text input behavior is preserved"; no new user-facing behaviour requested |
| Observable surfaces enumerable | PASS — listed below from live call sites |
| Test/render harness exists per surface | PASS — `components/editor_tests.rs`, `shell/app_input_paste_tests.rs`, `shell/app_event_loop_tests.rs`, rewind response tests, TUI BDD (paste/border/submit) |
| No mixed behaviour+refactor | PASS — pure restructure of input state/API; slash-command and modal selectors stay outside the system (open note on issue) |
| Performance of specialized code | WARNING accepted — see (c); consequence if regressed: history unbounded growth / extra render work / UTF-8 panics on mid-char cursor |

## Structural goals (REVIEW-TIME only — never test assertions)

- Clearly named text-input system/API used by the TUI (not ad-hoc field poking of buffer internals).
- Submitted-input history owned and managed by that system (not scattered across callers).
- Outside the system: mutation goes through the API; external access minimized to read/render accessors.
- API boundary documented (module docs / comments).
- Call sites in `shell` / rewind / harness use the system API rather than constructing parallel history or editing private buffer fields.

**Scope decision (issue open questions):** this system owns **only the main prompt input field** (draft text, cursor, submit, clear, paste, history navigation). Slash-command autocomplete, `@files` autocomplete, model/effort/resume/rewind selectors, and panel focus remain shell/feature concerns that *call* the text-input API. Modal selectors keep their own widgets.

## (a) Touched observable surfaces

1. **Draft text buffer** — multi-line content, join with `\n`, empty = single empty line
2. **Cursor** — row + byte column on char boundaries; insert/delete/move/home/end/word nav
3. **Submit** — Enter on non-whitespace-trimmed text; returns full text (not trimmed); clears draft; records history from **trimmed** text
4. **Clear / cancel** — `set_text("")` / Ctrl+C / Escape paths that empty the editor
5. **History navigation** — Up/Down when single-line draft; save/restore in-progress draft; no-dup last entry; cap 500
6. **Paste** — `\n`, `\r`, `\r\n` → newlines; never auto-submit
7. **Bash-mode indicator** — first line `trim_start().starts_with('!')` → yellow ` ! ` border vs accent ` > `
8. **Render** — top/bottom borders, padding, reverse-video block cursor when shown, width clamp, render cache by width
9. **Cursor visibility** — hidden when panel focus; no reverse-video when hidden
10. **Token replace** — `replace_before_cursor` for `@files` accept
11. **App integration** — key routing, autocomplete update from text, submit dispatch, rewind apply/baseline, harness accessors
12. **Autocomplete Enter path** — set text, `add_to_history(trim)`, submit, clear (today's sequence)

## (b) Per-surface behaviours that must stay identical

### Draft text / set_text / text
- `text()` joins lines with `\n` (no trailing newline for single empty line → `""`)
- `set_text("")` → one empty line; cursor row 0 col 0
- `set_text` with content splits on `\n`; cursor at end of last line
- Multi-line preserved exactly (including empty middle lines)

### Typing / deletion / cursor
- Insert at cursor; UTF-8 safe (char boundaries)
- Backspace: delete previous char, or join with previous line at col 0
- Delete: delete next char, or join with next line at EOL
- Left/Right wrap across lines; Up/Down move rows clamping col to line len
- Home/End and Ctrl+A/Ctrl+E: line start/end
- Ctrl+U kill-to-start; Ctrl+K kill-to-end
- Alt+b / word-left; Alt+f / word-right (ASCII whitespace word boundaries)
- Shift+Enter / Alt+Enter / Alt+`\r` insert newline (do not submit)
- Unhandled keys return `false` from `handle_input`

### Submit
- Empty or whitespace-only: no submit, draft unchanged, no history entry
- Non-empty after trim: `take_submit()` yields **untrimmed** full text; draft cleared; history gets **trimmed** entry
- Enter triggers submit when not captured by autocomplete/files/panel overlays

### History
- `add_to_history`: ignore empty; skip if equal to last entry; push; if len > 500 remove index 0; reset `history_index` to -1
- Up (single-line only): first Up saves current draft to `saved_text`, jumps to last history entry; further Up decrements index; at 0 stays
- Down: increments toward newest; past newest restores `saved_text` and index -1
- Empty history: Up no-op
- Multi-line draft: Up/Down move cursor rows, not history
- Submit always calls `add_to_history(trimmed)` then clears

### Paste
- Characters inserted; `\r\n` / `\r` / `\n` become newlines via insert_newline
- Trailing newline in paste does **not** submit (app paste tests)

### Bash mode / render
- Border color: yellow when bash mode else accent
- Indicator: ` ! ` vs ` > `
- Top border = remaining-width dashes around indicator; bottom full-width dashes
- Content lines padded with single space each side; truncate_to_width
- Cursor: reverse video `\x1b[7m`…`\x1b[27m` on char under cursor (or space at EOL)
- `set_show_cursor(false)`: no reverse video; toggle invalidates cache
- Cache: reuse when width matches; invalidate on content/cursor/show_cursor changes
- Defensive: mid-char cursor_col snaps to previous boundary in render helper

### replace_before_cursor
- Replaces `[start_col, cursor_col)` on current line with replacement; cursor after replacement
- No-op if start/end not char boundaries

### App-level (must remain identical outcomes)
- Ctrl+C: clear editor if non-empty else abort if running else noop
- Escape (master idle): clear non-empty editor; empty → rewind idle escape path
- After every editor key: autocomplete.update(text); files autocomplete from current_line+cursor_col unless slash active
- take_submit → handle_submit
- Slash autocomplete Enter: set_text(value), add_to_history(trim), dismiss, handle_submit, set_text("")
- Rewind apply: if editor text still equals pending baseline, set_text(rewound); else keep newer draft
- Render stack: editor after spinner/autocomplete; show_cursor iff focus != Panel

## (c) Performance characteristics (must not regress)

| Code | Characteristic |
|---|---|
| History store | `Vec<String>`; cap 500 via `remove(0)` when over (O(n) shift rare) |
| History nav | O(1) index + clone of selected entry into buffer via set_text |
| Render cache | Optional cached lines keyed by width; skip rebuild on hit |
| Insert/delete | In-place `String` ops on current line; line join/split allocates one String |
| Paste | Single pass over chars; no intermediate full-string rebuild beyond line ops |
| UTF-8 boundaries | O(1) amortized walk to nearest boundary |

## Characterization assets (pre-refactor)

Primary behavioural pins (existing — will freeze after mutation evidence):

| Path | Role |
|---|---|
| `quecto-tui/src/components/editor_tests.rs` | Unit: type/edit/cursor/submit/history/paste/render/utf8/cursor hide |
| `quecto-tui/src/shell/app_input_paste_tests.rs` | App: paste does not auto-submit |
| `quecto-tui/src/shell/app_event_loop_tests.rs` | App: keys, clear, @files replace, escape |
| `quecto-tui/src/conversation/app_rewind_response_tests.rs` | Rewind editor baseline/apply |
| TUI BDD paste/border/submit scenarios | End-to-end harness |

Freeze hashes recorded at characterize/freeze step after any hollow-assertion fixes.

## Out of scope (explicit)

- Changing history persistence across sessions (history is in-memory only today)
- Owning slash-command or modal selector widgets
- New keybindings or submit semantics
- Persist history to disk
