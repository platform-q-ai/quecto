# #1679 P4 slice 2 — UDS admission projection (verification)

Scope (notes/1679-p4-scope.md item 2): an orthogonal `admission` object on
`get_state`, the `waiting` progress verdict, `admission_state_changed` push,
freshness and delta bounds through the existing `generation` cursor, abort
while waiting, protocol docs. Built on slice 1 (#1700) and after #1698.

RED first: `src/domain/inference_admission_view_tests.rs`,
`src/interface/cli/uds_admission_projection_tests.rs` and the BDD feature
`inference_admission_projection.feature` failed to compile / matched no step
against the slice 1 tree (no `waiting_verdict`, `AdmissionSnapshot`,
`set_admission_source`, `AgentEvent::AdmissionStateChanged`,
`AdmissionStateProbe`). GREEN: 4 domain tests (no verdict without a waiter,
longest waiter names group and cause, cooldown states → causes with remaining
time, fully hidden queue reports an unknown wait), 6 interface tests (bounded
projection with cooldowns/counters/hidden, verdict text, waiting beats
quiet/active/advancing and phase untouched, revision folded into the cursor
exactly once, slim `get_state` carries `admission` and `since` sees
transitions, hook broadcasts one event per transition and tolerates no
client), 4 BDD scenarios over a real in-process authority (waiting verdict,
since-cursor freshness, abort cancels the wait, push events with increasing
revisions).

Design: domain `inference_admission_view::waiting_verdict` is a pure
projection of `AdmissionActivity` (ADR-0026 vocabulary, no text); the
interface `uds_admission_projection` owns the wire shape and reason text;
`ExecutionState` holds the `AdmissionObservation` read port, folds its
revision into the single public cursor in `observe_visible_revisions` (before
the snapshot, so a racing transition is folded by the next poll rather than
hidden) and ranks `waiting` above the tool-window verdicts; the
`ProcessAdmission::on_transition` hook broadcasts `admission_state_changed`
outside the recorder lock. Admission never appears in `state`.

## Mutations (slice 2, round 1)
    M1 verdict never produced: killed (inference_admission_view tests)
    M2 waiting does not out-rank tool verdicts: killed (a_waiting_attempt_is_reported_as_waiting_never_quiet_or_active)
    M3 admission revision not folded into the cursor: killed (a_changed_admission_revision_advances_the_public_cursor_once_per_observation)
    M4 cooldown remaining ignores observed time: killed (cooldown_states_become_causes_with_remaining_time)
    M5 shortest wait chosen: killed (the_longest_waiting_attempt_names_the_group_and_cause)
    M6 hook sends nothing: killed (the_hook_broadcasts_one_admission_state_changed_event_per_transition)
    M7 slim get_state drops admission: killed (slim_get_state_carries_admission_and_since_sees_transitions)
    M8 hidden count dropped from the projection: killed (projection_is_bounded_and_carries_cooldowns_and_counters)
    M9 seconds reported as milliseconds: killed (waiting_progress_names_count_group_wait_and_cause)
    M10 snapshot omits admission: dead-import compile error, superseded by M10b in round 2

## Adversarial review 1 (2026-09-09) and fixes

Twelve items (3 medium, 5 low, 3 info, 1 housekeeping), all fixed:

|Finding|Fix|Proof|
|---|---|---|
|M1 `admission_state_changed` could be delivered out of revision order under concurrent transitions (clients do a simple replace)|recorder serializes snapshot+hook behind a `notify` mutex distinct from the state lock; docs say "delivered in revision order, simple replace"|`concurrent_transitions_reach_the_hook_in_revision_order` (8 workers × 25 transitions)|
|M2 socket-loop wiring untested|wiring extracted to `attach_process_admission`, exercised over a real in-process authority|`attaching_a_process_binding_projects_and_pushes_its_transitions`|
|M3 docs claimed one cursor advance per transition; code advances once per observation|docs and test name state the real invariant: a changed revision advances `generation` once per observation, never `unchanged` across a transition|`a_changed_admission_revision_advances_the_public_cursor_once_per_observation`|
|L1 time-derived fields move under the `unchanged` marker|documented: `revision`/`generation` track transitions only; elapsed/cooldown re-derived on full reads; domain comment corrected|docs|
|L2 fully hidden queue attributed to an arbitrary group|`WaitingVerdict.attribution: Option<(GroupId, WaitCause)>`; reason says "beyond the sampled attempts"|`a_fully_hidden_queue_reports_an_unknown_wait_without_inventing_one`, `a_fully_hidden_queue_is_reported_without_a_guessed_group`|
|L3 `hidden` meaningless without the attempts list|re-documented as the sample bound behind `longestWaitSeconds` (may under-report when non-zero)|docs|
|L4 abort scenario scripted its own idle; hook installed after the first transition|abort step no longer finishes the run (post-abort poll asserts `active`, cancelled=1, nothing waiting); hook installed before queueing, first pushed event asserted as the queued transition, three events in order|BDD scenarios 3 and 4|
|L5 collapsible `if let`|let-chain|clippy|
|I1 hook keeps a sender alive in the process-wide binding|documented on `attach_process_admission`|—|
|I2 recorder `Debug` could deadlock under its own lock|`try_lock` in the impl|—|
|I3 uncommitted work|committed with the slice|—|
