# Issue #1586 phase 3 targeted verification

Commands run before commit:

```sh
cargo fmt --all --check
cargo clippy -q -p quecto-agentic-harness -p quecto-tui --all-targets -- -D warnings
# Targeted tests, one cargo invocation per test name:
cargo test -q -p quecto-agentic-harness test_parse_persist_session_ordinary_exit_barrier -- --nocapture
cargo test -q -p quecto-agentic-harness test_parse_persist_session_unknown_reason_without_id -- --nocapture
cargo test -q -p quecto-agentic-harness persist_session_unknown_restore_reason_matches_omitted_legacy_behavior -- --nocapture
cargo test -q -p quecto-agentic-harness persist_session_omitted_restore_reason_uses_legacy_behavior -- --nocapture
cargo test -q -p quecto-agentic-harness persist_session_empty_roster_replaces_stale_same_session_only -- --nocapture
cargo test -q -p quecto-agentic-harness persist_session_non_empty_roster_replaces_stale_same_session_only -- --nocapture
cargo test -q -p quecto-agentic-harness persist_session_dispatch_success_emits_correlated_ok_event -- --nocapture
cargo test -q -p quecto-agentic-harness persist_session_dispatch_failure_emits_correlated_err_event -- --nocapture
cargo test -q -p quecto-agentic-harness ordinary_exit_snapshot_marks_dead_tombstones_non_restorable -- --nocapture
cargo test -q -p quecto-tui ordinary_exit_fanout_targets_all_sendable_tabs_without_focus_or_name_collapse -- --nocapture
cargo test -q -p quecto-tui ordinary_exit_fanout_continues_after_first_enqueue_failure -- --nocapture
```

Result: PASS. Full output: `/tmp/fmt_clippy.out`, `/tmp/target_tests2.out`.

AC/matrix evidence:

- Explicit `persist_session` protocol command: `test_parse_persist_session_ordinary_exit_barrier`, `test_parse_persist_session_unknown_reason_without_id`.
- Unknown/omitted restore reason compatibility and legacy mapping: `persist_session_unknown_restore_reason_matches_omitted_legacy_behavior`, `persist_session_omitted_restore_reason_uses_legacy_behavior`.
- Ordinary-exit restore-reason classifier behavior and dead tombstone non-restorability: `ordinary_exit_snapshot_marks_dead_tombstones_non_restorable`.
- Current roster replaces stale same-session roster without affecting other sessions: `persist_session_empty_roster_replaces_stale_same_session_only`, `persist_session_non_empty_roster_replaces_stale_same_session_only`.
- Harness dispatch success/failure observability and request correlation: `persist_session_dispatch_success_emits_correlated_ok_event`, `persist_session_dispatch_failure_emits_correlated_err_event`.
- TUI fan-out targets all sendable/open tabs independent of focus/name and preserves correlation: `ordinary_exit_fanout_targets_all_sendable_tabs_without_focus_or_name_collapse`.
- Enqueue-failure continuation and manifest flush: `ordinary_exit_fanout_continues_after_first_enqueue_failure`.
- Formatting/clippy/compile hygiene: `cargo fmt --all --check`, targeted strict clippy for changed packages with all targets.
