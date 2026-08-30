# Issue #1586 phase 3 test/check design

Traceability: frozen matrix in `notes/issue-1586-phase3-scope.md`.

## Required tests/checks

1. `protocol_tests::test_parse_persist_session_ordinary_exit_barrier`
   - Covers: explicit tab snapshot command; protocol/additive compatibility; correlation id; ordinary-exit restore reason input.
   - Scenario: deserialize `{type:"persist_session", id:"tab1:persist-exit", restoreReason:"ordinary_tui_exit_stopped"}`.
   - Expect: command variant is `PersistSession`, id is preserved, reason is preserved.

2. `persist_session_unknown_restore_reason_matches_omitted_legacy_behavior` (add)
   - Covers: negative compatibility for unknown restore reason.
   - Scenario: dispatch or invoke persist with unknown `restoreReason` while registry has a restorable row, and compare with the same setup omitting `restoreReason`.
   - Expect: both commands succeed/do not reject protocol; persisted roster state is identical for unknown and omitted reason; neither stamps rows as `OrdinaryTuiExitStopped`.

3. `persist_session_omitted_restore_reason_uses_legacy_behavior` (add)
   - Covers: omitted-field compatibility from the frozen matrix.
   - Scenario: deserialize and dispatch/invoke `persist_session` without `restoreReason` while registry has a restorable row.
   - Expect: command succeeds, request id is preserved, and persisted roster is not stamped `OrdinaryTuiExitStopped` (legacy/default behavior).

3. Existing harness roster classifier test: `ordinary_exit_snapshot_marks_dead_tombstones_non_restorable`
   - Covers: phase-2 classifier use; ordinary-exit stamping; dead/tombstone non-restorability.
   - Scenario: snapshot with ordinary-exit reason and dead tombstone rows.
   - Expect: live/restorable rows carry ordinary-exit reason; dead/killed rows do not become restorable.

4. `persist_session_empty_roster_replaces_stale_same_session_only` (add)
   - Covers: current roster replaces stale roster; empty current roster clears stale rows for same session; unrelated sessions unaffected.
   - Scenario: create durable store containing session A with stale roster rows and session B with roster rows; invoke phase-3 persist path for session A with an empty current registry; reload storage.
   - Expect: session A roster is empty/current; session B data remains unchanged.
   - This must be executable, not inspection-only.

5. `ordinary_exit_barrier_targets_all_sendable_tabs_without_focus_or_name_collapse` (add if feasible; otherwise document exact construction blocker)
   - Covers: every open/sendable tab targeted independent of focus/display-name collisions; active tab unchanged.
   - Scenario: construct an app with at least two open/sendable tabs, duplicate display labels if fixture supports it, active tab not first; invoke `enqueue_ordinary_exit_snapshot_persists()`.
   - Expect observable behavior only: each sendable tab receives exactly one `persist_session` command, request ids are unique/correlatable to their tab connection, each command carries `ordinary_tui_exit_stopped`, and active tab is unchanged.
   - Do not assert helper calls such as `ordered_tab_ids`, `conn_for`, or traversal mechanics.

6. `ordinary_exit_barrier_continues_after_first_enqueue_failure_and_flushes_manifest` (add)
   - Covers: high-risk failure boundary; first tab enqueue failure must not prevent later tabs or manifest flush.
   - Scenario: first open tab has disconnected/full transport, second open tab is healthy, manifest path is observable; invoke barrier.
   - Expect: second tab receives `persist_session`; manifest file exists/was updated with the current workspace/tab snapshot after barrier despite the first enqueue failure; barrier returns the first enqueue error.
   - Must be executable because control-flow risk is high.

7. `persist_session_dispatch_failure_emits_correlated_err_event` (add)
   - Covers: harness persistence failure surfacing and initiator correlation.
   - Scenario: dispatch `persist_session` with request id against a persistence path forced to fail (for example unwritable/session-store error fixture).
   - Expect: emitted event is `err`, has type `persist_session`, and preserves request id.

8. `persist_session_dispatch_success_emits_correlated_ok_event` (add or pair with failure test)
   - Covers: successful command observable contract.
   - Scenario: dispatch `persist_session` for a durable session with a valid persistence path.
   - Expect: `ok` event has type `persist_session`, preserves request id, and saved session includes current roster snapshot.

9. Manifest/workspace durability positive check
   - Covers: barrier flushes workspace manifest alongside per-tab session persists, including missing parent directory.
   - Scenario: barrier invoked for workspace with current registry and missing durability parent directory.
   - Expect: manifest file exists and contains the expected/current workspace/tab snapshot after barrier.
   - Can be covered by test 6 if it uses real durability paths; otherwise add a focused manifest assertion.

10. Compatibility/regression check
   - Covers: additive protocol command and legacy sessions.
   - Scenario: existing commands and sessions without roster/reason still compile/test; omitted `restoreReason` deserializes and maps to legacy behavior.
   - Expect: no deserialization or compile regressions; omitted restore reason maps to legacy behavior.
   - Evidence: `cargo check`, focused test suite, and the explicit unknown/omitted reason tests above.

## Reviewer findings and resolution

- Empty-roster replacement cannot remain inspection-only: resolved by requiring executable test 4.
- Barrier enqueue-failure continuation cannot remain inspection-only: resolved by requiring executable test 6.
- Harness persist failure UDS error event cannot be conditional: resolved by requiring executable test 7.
- Unknown `restoreReason` compatibility needed concrete coverage: resolved by requiring executable test 2 with concrete equality to omitted-field behavior.
- Omitted `restoreReason` compatibility needed concrete coverage: resolved by requiring executable test 3.
- Manifest flushing needed an observable assertion rather than mock/attempt wording: resolved by requiring a real manifest file/current snapshot assertion in tests 6/9.
- All-tabs barrier test risked implementation-detail assertions: resolved by specifying only observable command delivery, correlation uniqueness, ordinary reason, and active-tab preservation.

## Out-of-scope assertions deliberately not tested in phase 3

- Actual Ctrl-D, `/exit`, `/quit` invocation of barrier (phase 4).
- Parent/subagent termination ordering or abort/proceed policy after failure (phase 4).
- Resume UX and sendability semantics for persisted ordinary-exit rows (phase 5).
- Docs/help copy (phase 6).
