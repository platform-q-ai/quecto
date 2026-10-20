//! The running authority's session policy (#1910): one settling turn, a
//! connection binding its scope, and a connection ending. The broker's actor
//! invokes these use cases and keeps only its transport bookkeeping.
use super::{authority, group, wrong};
use quecto::application::admission::dto::broker::{AuthorityError, Credential, Settlement};
use quecto::domain::admission::value_objects::inference_admission::{
    AdmissionError, Feedback, RequestId, RequestState, TerminalOutcome, WorkloadClass,
};

fn id(credential: &Credential, sequence: u64) -> RequestId {
    RequestId {
        scope: credential.scope,
        sequence,
    }
}

#[test]
fn settling_answers_grants_and_leaves_queued_waiters_unanswered() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.acquire(&root, 2, "g", 0).unwrap();
    let settlement = a.settle(&[id(&root, 1), id(&root, 2)], 0);
    assert_eq!(settlement.dispatch_failure, None);
    assert!(settlement.journal_healthy);
    assert_eq!(settlement.answers.len(), 1, "the queued waiter waits on");
    let (answered, answer) = &settlement.answers[0];
    assert_eq!(*answered, id(&root, 1));
    assert!(
        matches!(answer, Ok(RequestState::Active { .. })),
        "{answer:?}"
    );
    assert!(
        settlement.wake_at(0).is_some(),
        "the queued waiter's deadline wakes the authority"
    );
}

#[test]
fn a_withdrawn_waiter_is_answered_journal_unavailable_while_the_journal_is_unhealthy() {
    let (mut a, journal) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    journal.log().fail = true;
    let settlement = a.settle(&[id(&root, 1)], 0);
    assert_eq!(
        settlement.dispatch_failure,
        Some(AuthorityError::JournalUnavailable)
    );
    assert!(!settlement.journal_healthy);
    assert_eq!(
        settlement.answers,
        vec![(id(&root, 1), Err(AuthorityError::JournalUnavailable))],
        "withdrawn by dispatch, not cancelled by its client"
    );
}

#[test]
fn a_client_cancel_is_answered_cancelled_while_the_journal_is_healthy() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.acquire(&root, 2, "g", 0).unwrap();
    a.cancel(&root, 2, 1).unwrap();
    let settlement = a.settle(&[id(&root, 2)], 1);
    assert_eq!(
        settlement.answers,
        vec![(
            id(&root, 2),
            Ok(RequestState::Terminal(TerminalOutcome::Cancelled))
        )]
    );
}

#[test]
fn an_admission_refusal_is_routine_and_not_a_dispatch_failure() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.disconnect(root.scope, 1).unwrap();
    assert_eq!(
        a.pump(1).err(),
        Some(AuthorityError::Admission(AdmissionError::Quarantined)),
        "the group is quarantined"
    );
    assert_eq!(a.settle(&[], 2).dispatch_failure, None);
}

#[test]
fn a_disconnect_with_nothing_unverified_releases_the_scope() {
    let (mut a, _) = authority();
    let holder = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&holder, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    let report = a.disconnect(root.scope, 1).unwrap();
    assert_eq!(
        (report.cancelled, report.uncertain, report.active),
        (1, 0, 0)
    );
    assert_eq!(
        a.status(&root, 1, 2).err(),
        Some(AuthorityError::Unauthorized),
        "the released capability is gone"
    );
    assert_eq!(a.inspect(2).live_scopes, 1);
}

#[test]
fn a_disconnect_with_unverified_work_keeps_the_capability() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    let report = a.disconnect(root.scope, 1).unwrap();
    assert_eq!(
        (report.cancelled, report.uncertain, report.active),
        (0, 1, 1)
    );
    assert!(
        matches!(a.status(&root, 1, 2), Ok(RequestState::Active { .. })),
        "the same capability may reconnect and complete its work"
    );
    assert_eq!(a.inspect(2).groups[&group()].uncertain, 1);
}

#[test]
fn binding_answers_the_sequence_after_the_high_water() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    assert_eq!(a.bind(&root, false, 0).unwrap(), 1);
    a.acquire(&root, 3, "g", 0).unwrap();
    assert_eq!(a.bind(&root, false, 1).unwrap(), 4);
}

#[test]
fn a_superseding_bind_disconnects_the_scope_first() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.acquire(&root, 2, "g", 0).unwrap();
    assert_eq!(a.bind(&root, true, 1).unwrap(), 3);
    assert_eq!(
        a.status(&root, 2, 1).unwrap(),
        RequestState::Terminal(TerminalOutcome::Cancelled),
        "the superseded connection's queued work can never be answered"
    );
    assert_eq!(a.inspect(1).groups[&group()].uncertain, 1);
}

#[test]
fn a_plain_or_forged_bind_disconnects_nothing() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.acquire(&root, 2, "g", 0).unwrap();
    assert_eq!(
        a.bind(&wrong(&root), true, 1).err(),
        Some(AuthorityError::Unauthorized)
    );
    a.bind(&root, false, 1).unwrap();
    assert!(
        matches!(a.status(&root, 2, 1).unwrap(), RequestState::Queued { .. }),
        "queued work is untouched"
    );
    assert_eq!(a.inspect(1).groups[&group()].uncertain, 0);
}

#[test]
fn an_idle_authority_sleeps_until_work_arrives() {
    let (mut a, _) = authority();
    assert_eq!(a.settle(&[], 0).wake_at(0), None);
}

#[test]
fn a_journal_outage_is_probed_at_a_bounded_cadence() {
    let (mut a, journal) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    journal.log().fail = true;
    let settlement = a.settle(&[], 10);
    assert!(!settlement.journal_healthy);
    assert!(
        settlement.wake_at(10).is_some_and(|at| at <= 10 + 250),
        "probe within the interval, not at the queue deadline: {:?}",
        settlement.wake_at(10)
    );
}

/// Review MED (#2625): a scope that reconnected and dropped again still
/// owes its unverified work, so the second disconnect keeps it rather than
/// trying (and failing) to release it.
#[test]
fn a_second_disconnect_keeps_a_scope_that_still_owes_work() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    assert_eq!(a.disconnect(root.scope, 1).unwrap().active, 1);
    a.bind(&root, false, 2).unwrap();
    let again = a
        .disconnect(root.scope, 3)
        .expect("a normal flow, not an error");
    assert_eq!(
        (again.uncertain, again.active),
        (0, 1),
        "already unverified"
    );
    assert!(
        matches!(a.status(&root, 1, 4), Ok(RequestState::Active { .. })),
        "the capability is kept for the work it owes"
    );
    a.complete(&root, 1, Feedback::Success, 5).unwrap();
    let last = a.disconnect(root.scope, 6).unwrap();
    assert_eq!(last.active, 0);
    assert_eq!(
        a.status(&root, 1, 7).err(),
        Some(AuthorityError::Unauthorized),
        "with nothing owed, the scope is released"
    );
}

/// Review LOW (#2625): a superseding bind whose disconnect fails is refused,
/// and the scope's work is untouched.
#[test]
fn a_superseding_bind_whose_disconnect_fails_is_refused() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(5).unwrap();
    a.acquire(&root, 2, "g", 5).unwrap();
    assert_eq!(
        a.bind(&root, true, 1).err(),
        Some(AuthorityError::Admission(AdmissionError::TimeRegression)),
        "an earlier clock cannot disconnect"
    );
    assert!(
        matches!(a.status(&root, 2, 6).unwrap(), RequestState::Queued { .. }),
        "queued work is untouched"
    );
    assert_eq!(a.inspect(6).groups[&group()].uncertain, 0);
    assert_eq!(
        a.bind(&root, false, 1).unwrap(),
        3,
        "a plain bind reads no clock"
    );
}

fn settlement(next_wake: Result<Option<u64>, AuthorityError>, journal_healthy: bool) -> Settlement {
    Settlement {
        dispatch_failure: None,
        answers: Vec::new(),
        cancellation_due: Vec::new(),
        next_wake,
        journal_healthy,
    }
}

/// Review MED (#2625 round 2): the next turn is measured from when the
/// broker arms its timer, and the retry and probe intervals are pinned.
#[test]
fn the_next_turn_is_measured_from_arming() {
    let armed = 1_000;
    assert_eq!(settlement(Ok(None), true).wake_at(armed), None, "idle");
    assert_eq!(
        settlement(Ok(Some(5_000)), true).wake_at(armed),
        Some(5_000)
    );
    assert_eq!(
        settlement(Err(AuthorityError::JournalUnavailable), true).wake_at(armed),
        Some(armed + 50),
        "a policy that cannot say retries 50 ms after arming"
    );
    assert_eq!(
        settlement(Ok(None), false).wake_at(armed),
        Some(armed + 250),
        "an outage probes 250 ms after arming"
    );
    assert_eq!(
        settlement(Ok(Some(armed + 10)), false).wake_at(armed),
        Some(armed + 10),
        "an earlier policy wake wins"
    );
    assert_eq!(
        settlement(Ok(Some(armed + 10_000)), false).wake_at(armed),
        Some(armed + 250),
        "a later one is clamped to the probe"
    );
    assert_eq!(
        settlement(Err(AuthorityError::JournalUnavailable), false).wake_at(armed),
        Some(armed + 50),
        "the retry is sooner than the probe"
    );
}
