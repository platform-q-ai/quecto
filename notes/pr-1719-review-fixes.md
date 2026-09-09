# PR #1719 review fixes

Reviewed submitted review 5159638042 and both inline findings, issue #1708,
and exact fetched head 64eafc9a6e066a724d63cd8d14fcb0bc36f4d00d before edits.

Acceptance checklist:
- Keep expiry repaint armed when select is rebuilt after expiry; become idle after service.
- After child TurnEnd, project cooldown through expiry without restoring waiting or mutating the snapshot.

RED: `cargo test -p quecto-tui --lib admission` produced 26 passes and two failures:
`expiry_guard_keeps_final_projection_armed_after_select_reentry` failed the final-repaint guard;
`child_turn_end_clears_wait_but_preserves_cooldown` failed the post-terminal service assertion.
These assert observable labels and the actual scheduling guard, using paused time; removing the respective fixes reproduces these failures.

GREEN/refactor: shared child projection applies the terminal wait-only overlay; scheduling checks active clocks or pending label differences. No separate pending flag can drift out of sync.
The same command passes all 28 tests. Full `cargo test -p quecto-tui --lib` passes 2186 tests.
`cargo fmt --all` completed.

Sweep: inspected all admission_unversioned_clears and has_active_dated_cooldown_after uses.
Master terminal clearing already rebases its projected snapshot. Child guard and tick now share projection semantics.
Out-of-scope alias-rekey overlay identity defect filed as #1723.
