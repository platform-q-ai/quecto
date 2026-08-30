# Issue #1586 phase 3 RED/falsifiability evidence

All new/changed phase-3 assertions are either behavioural tests added during implementation or compatibility/protocol characterizations. Behavioural tests were added after the implementation existed, so falsifiability was demonstrated by targeted mutation/review rather than preserving failing commits.

## Protocol command serialization/deserialization

Test:
- `cargo test -q -p quecto-agentic-harness test_parse_persist_session_ordinary_exit_barrier -- --nocapture`

Temporary mutation:
- Changed only `AgentCommand::PersistSession` serde variant name from `persist_session` to `persist_session_MUTATION`.

Expected/observed failure:
- `serde_json::from_str` failed with `unknown variant persist_session`, proving the test rejects command renaming/removal.

Mutation residue check after revert:
- `git diff -- quecto-agentic-harness/src/interface/cli/protocol_commands.rs | grep MUTATION || true`

## Unknown/omitted restore reason compatibility

Tests:
- `cargo test -q -p quecto-agentic-harness persist_session_unknown_restore_reason_matches_omitted_legacy_behavior -- --nocapture`
- `cargo test -q -p quecto-agentic-harness persist_session_omitted_restore_reason_uses_legacy_behavior -- --nocapture`

Falsifiability evidence:
- Before the review fix that normalizes `SubagentRestoreReason::Unknown` to `LegacyUnspecified` in the persist path, `persist_session_unknown_restore_reason_matches_omitted_legacy_behavior` failed with `left: Unknown, right: LegacyUnspecified`.
- The omitted-field test would fail if default/legacy reason mapping stamped `OrdinaryTuiExitStopped` or rejected missing `restoreReason`.

## Empty roster replacement / stale-row clearing

Test:
- `cargo test -q -p quecto-agentic-harness persist_session_empty_roster_replaces_stale_same_session_only -- --nocapture`

Falsifiability evidence:
- Test constructs stale durable roster for session A and unrelated session B, invokes the phase-3 persist path with an empty current registry, then reloads storage.
- It fails if persist appends instead of replaces, fails to force full save for command-driven persist, or mutates unrelated sessions.

## Harness dispatch ok/error observability

Tests:
- `cargo test -q -p quecto-agentic-harness persist_session_dispatch_success_emits_correlated_ok_event -- --nocapture`
- `cargo test -q -p quecto-agentic-harness persist_session_dispatch_failure_emits_correlated_err_event -- --nocapture`

Falsifiability evidence:
- Success test fails if dispatch does not emit a `response` event with `success:true`, `command:"persist_session"`, preserved request id, or if the session is not saved.
- Failure test creates a blocking `sessions` file under the temp store; it fails if persistence errors are swallowed, reported as success, lose request id/type, or do not include an error string.

## TUI fan-out and enqueue-failure continuation

Tests:
- `cargo test -q -p quecto-tui ordinary_exit_fanout_targets_all_sendable_tabs_without_focus_or_name_collapse -- --nocapture`
- `cargo test -q -p quecto-tui ordinary_exit_fanout_continues_after_first_enqueue_failure -- --nocapture`

Falsifiability evidence:
- The all-tabs test observes real command-channel output and fails if only the active tab is targeted, command ids collide, ordinary-exit reason is omitted/wrong, or active tab changes.
- The failure-continuation test uses a disconnected first tab and healthy second tab; it fails if the fan-out returns early, skips later tabs, omits manifest persistence, or fails to return the enqueue error.

## Current target verification command set

- `cargo test -q -p quecto-agentic-harness test_parse_persist_session_ordinary_exit_barrier -- --nocapture`
- `cargo test -q -p quecto-agentic-harness persist_session_unknown_restore_reason_matches_omitted_legacy_behavior -- --nocapture`
- `cargo test -q -p quecto-agentic-harness persist_session_omitted_restore_reason_uses_legacy_behavior -- --nocapture`
- `cargo test -q -p quecto-agentic-harness persist_session_empty_roster_replaces_stale_same_session_only -- --nocapture`
- `cargo test -q -p quecto-agentic-harness persist_session_dispatch_success_emits_correlated_ok_event -- --nocapture`
- `cargo test -q -p quecto-agentic-harness persist_session_dispatch_failure_emits_correlated_err_event -- --nocapture`
- `cargo test -q -p quecto-agentic-harness ordinary_exit_snapshot_marks_dead_tombstones_non_restorable -- --nocapture`
- `cargo test -q -p quecto-tui ordinary_exit_fanout_targets_all_sendable_tabs_without_focus_or_name_collapse -- --nocapture`
- `cargo test -q -p quecto-tui ordinary_exit_fanout_continues_after_first_enqueue_failure -- --nocapture`
- `cargo check -q -p quecto-agentic-harness -p quecto-tui`
