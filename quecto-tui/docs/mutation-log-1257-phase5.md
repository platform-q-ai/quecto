# Mutation log — #1257 Phase 5 characterization falsifiability

All mutations applied to production code, observed FAIL, then fully reverted.
Final `git diff` after log: empty (no residue).

| # | Assertion / test | Mutation | Observed failure |
|---|---|---|---|
| 1 | `app_workspace_file_autocomplete_uses_production_visible_capacity` | `FilesAutocomplete::new(8)` → `new(7)` in `app_workspace.rs` | FAIL: expected 8 visible rows |
| 2 | `footer_apply_get_state_shows_effort_level` | Drop effort mapping in `Footer::apply_get_state` | FAIL: effort level not shown |
| 3b | `is_safe_path_rejects_control_chars` | `is_safe_path` allows control chars | FAIL: control path accepted |
| 4 | `main_pane_compact_line_reflects_live_auto_continue_state` | Ignore `autoContinue` in `sync_workflow_automation` | FAIL: compact line stays `auto:off` |
| 5 | `read_git_branch_reflects_head_changes_without_restart` | Strip `refs/tags/` instead of `refs/heads/` | FAIL: branch name mismatch |
| 6 | `model_field_wins_over_id_field` | Prefer `id` over `model` in mapper | FAIL: wrong id selected |
| 7 | `footer_updates_when_effort_changes` | Skip `footer.set_effort` on set_effort success | FAIL: footer effort display stale |
| 7a | (hollow, declined) | Drop only `current_effort` field assignment | PASS — test observes footer render, not private field; covered by #7 footer path |

Note: Mutating `MAX_WORKSPACE_FILES` alone does not fail `parse_git_output_caps_at_max` because that test reads the constant — capacity behaviour is still pinned by the constant + cap loop; 3b covers the safety filter invariant instead.

Suite restored GREEN after each revert.
