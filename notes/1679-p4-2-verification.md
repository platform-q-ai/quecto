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
