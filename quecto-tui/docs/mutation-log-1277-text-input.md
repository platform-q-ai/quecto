# Mutation log — #1277 text input characterization

Every mutation was applied to production code, verified to fail the named
characterization test, then reverted. Final residue check: production files
match pre-mutation baseline; suites GREEN.

## Round 1 — editor unit gaps (editor.rs)

| ID | Mutation | Test | Observed failure |
|---|---|---|---|
| M1 | Skip empty/whitespace submit guard | `submit_whitespace_only_does_not_submit_or_clear` | `Some("   \t  ")` vs `None` |
| M2 | submit stores trimmed | `submit_returns_untrimmed_text_and_records_trimmed_history` | `Some("hello")` vs `Some("  hello  ")` |
| M3 | history stores untrimmed | same | Up restored padded text |
| M4 | Remove empty-submit early return | `empty_submit_is_noop` | `Some("")` vs `None` |
| M5 | Drop last-entry dedupe | `history_skips_duplicate_of_last_entry` | Down path length wrong |
| M6 | Clear saved draft on first history Up | `history_up_saves_in_progress_draft_and_down_restores_it` | Down restored `""` |
| M7 | Disable history cap eviction | `history_cap_evicts_oldest_beyond_500` | oldest `entry-0` not `entry-1` |
| M8 | Up always navigates history | `multiline_up_does_not_navigate_history` | text became history entry |
| M9 | Empty history Up clears draft | `empty_history_up_is_noop` | text `""` |
| M10 | Bash mode without `trim_start` | `bash_mode_after_leading_whitespace_on_first_line` | bash false |
| M11 | Normal indicator `" ! "` | `normal_mode_prompt_indicator_is_gt` | no `>` |
| M12 | set_text cursor at (0,0) | `set_text_multiline_places_cursor_at_end_of_last_line` | wrong line |
| M13 | Skip char-boundary guard in replace | `replace_before_cursor_noop_on_non_boundary_is_safe` | panic mid-char |
| M14 | Drop `Alt('\r')` from newline arm | `alt_carriage_return_inserts_newline` | `"ab"` vs `"a\nb"` |
| M15 | `MAX_HISTORY = 499` | `history_cap_keeps_oldest_at_exactly_500_entries` | oldest `entry-1` |

Hollow assertion fixed before freeze: initial M5 walk stayed on duplicate text;
rewritten to assert Down path length (two entries only).

## Round 2 — app integration (app_event_loop.rs) after coverage finder

| ID | Mutation | Test | Observed failure |
|---|---|---|---|
| M16 | Skip `autocomplete.update` after editor keys | `handle_key_editor_input_updates_slash_autocomplete_from_editor_text` | autocomplete not active |
| M17 | Drop `handle_submit` after `take_submit` | `handle_key_enter_with_nonempty_editor_submits_prompt_and_clears` | no user entry |
| M18 | Skip `add_to_history` on slash Enter | `slash_autocomplete_enter_accepts_submits_history_and_clears` | Up restores `""` |

## Finder triage (characterization)

| Finding | Disposition |
|---|---|
| Falsifiability: no findings | Accepted |
| Coverage MED: autocomplete update / Enter submit / slash Enter | Fixed with new tests + M16–M18 |
| Coverage LOW: history exactly 500 / Alt+`\r` | Fixed (M14–M15) |
| Coverage LOW: border colors, EOL cursor space, panel focus→cursor | **Declined** — already covered at widget level (`hidden_cursor`, bash indicator); color/EOL are pure render cosmetics with existing border/width pins; panel focus is `set_show_cursor` call site with cursor-hide pin |
| Gherkin: no #1277 BDD added | **Accepted** — unit characterization is behavioural |
| Gherkin MED: rewind When bundles two actions | **Declined for this slice** — pre-existing BDD outside #1277 freeze; not introduced by characterization |
| Gherkin LOW: autocomplete Enter wording | **Declined** — pre-existing; BDD already asserts submit outcomes |
