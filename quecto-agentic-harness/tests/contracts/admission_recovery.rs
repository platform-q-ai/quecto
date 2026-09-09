//! AdmissionRecovery contract (#1679 P3 AC1/AC6): abandonment, quarantine,
//! withdrawal, dispatch wake, durable ledger and operator epoch reset.
use quecto::application::inference_admission::AdmissionService;
use quecto::application::ports::{
    AdmissionClient, AdmissionDispatcher, AdmissionRecovery, AdmissionRegistry,
};
use quecto::domain::inference_admission::{
    AdmissionConfig, AdmissionError, Feedback, GroupId, GroupPolicy, RequestId, RequestState,
    ScopeId, TerminalOutcome, WorkloadClass,
};
use std::collections::BTreeMap;

fn policy(capacity: usize) -> GroupPolicy {
    GroupPolicy {
        capacity,
        reserve: 0,
        min_interval_ms: 10,
        queue_capacity: 4,
        queue_timeout_ms: 1_000,
        attempt_timeout_ms: 5_000,
        fallback_base_ms: 100,
        max_cooldown_ms: 10_000,
    }
}

fn config() -> AdmissionConfig {
    let a = GroupId::new("a").unwrap();
    let b = GroupId::new("b").unwrap();
    AdmissionConfig {
        groups: BTreeMap::from([(a.clone(), policy(2)), (b.clone(), policy(1))]),
        aliases: BTreeMap::from([("a".into(), a), ("b".into(), b)]),
        max_scopes: 8,
        terminal_capacity: 16,
    }
}

fn group(name: &str) -> GroupId {
    GroupId::new(name).unwrap()
}

/// Two roots; root one owns one active attempt (seq 1) and one queued (seq 2)
/// in group `a`; root two has one queued attempt in `a`.
fn seeded() -> (AdmissionService, ScopeId, ScopeId) {
    let mut s = AdmissionService::new(3, config()).unwrap();
    let one = s.register_root(WorkloadClass::Interactive).unwrap();
    let two = s.register_root(WorkloadClass::Interactive).unwrap();
    s.enqueue(one, 1, "a", 0).unwrap();
    assert_eq!(
        s.next(&group("a"), 0).unwrap(),
        Some(RequestId {
            scope: one,
            sequence: 1
        })
    );
    s.enqueue(one, 2, "a", 1).unwrap();
    s.enqueue(two, 1, "a", 1).unwrap();
    (s, one, two)
}

#[test]
fn abandon_cancels_queued_work_so_it_never_dispatches() {
    let (mut s, one, two) = seeded();
    let report = s.abandon(one, 2).unwrap();
    assert_eq!(report.cancelled, 1, "queued attempt cancelled");
    assert_eq!(
        s.status(one, 2, 2).unwrap(),
        RequestState::Terminal(TerminalOutcome::Cancelled)
    );
    let _ = two;
}

#[test]
fn abandon_marks_active_attempt_uncertain_and_quarantines_its_group() {
    let (mut s, one, _two) = seeded();
    let report = s.abandon(one, 2).unwrap();
    assert_eq!(report.uncertain, 1, "active attempt becomes uncertain");
    let snapshot = s.snapshot(&group("a"), 20).unwrap();
    assert_eq!(snapshot.uncertain, 1, "snapshot exposes uncertainty");
    assert_eq!(snapshot.active, 1, "uncertain attempt retains capacity");
    assert_eq!(
        s.next(&group("a"), 20),
        Err(AdmissionError::Quarantined),
        "quarantined group grants nothing even with free capacity"
    );
}

#[test]
fn quarantine_is_group_scoped() {
    let (mut s, one, two) = seeded();
    s.abandon(one, 2).unwrap();
    s.enqueue(two, 2, "b", 3).unwrap();
    assert_eq!(
        s.next(&group("b"), 3).unwrap(),
        Some(RequestId {
            scope: two,
            sequence: 2
        }),
        "independent group keeps dispatching"
    );
}

#[test]
fn verified_completion_by_the_same_scope_clears_quarantine() {
    let (mut s, one, two) = seeded();
    s.abandon(one, 2).unwrap();
    s.complete(one, 1, Feedback::Success, 30).unwrap();
    assert_eq!(s.snapshot(&group("a"), 30).unwrap().uncertain, 0);
    assert_eq!(
        s.next(&group("a"), 30).unwrap(),
        Some(RequestId {
            scope: two,
            sequence: 1
        }),
        "grants resume after verified termination"
    );
}

#[test]
fn retiring_a_scope_with_uncertain_work_is_refused() {
    let (mut s, one, _two) = seeded();
    s.abandon(one, 2).unwrap();
    assert_eq!(s.retire(one), Err(AdmissionError::Busy));
}

#[test]
fn reset_starts_a_successor_epoch_that_forgets_uncertainty_but_keeps_cooldown() {
    let (mut s, one, two) = seeded();
    s.abandon(one, 2).unwrap();
    s.enqueue(two, 2, "b", 3).unwrap();
    s.next(&group("b"), 3).unwrap();
    s.complete(two, 2, Feedback::Throttle { delay_ms: 500 }, 4)
        .unwrap();
    assert_eq!(s.reset(10).unwrap(), 4, "successor epoch");
    assert_eq!(
        s.status(one, 1, 10),
        Err(AdmissionError::StaleEpoch),
        "old-epoch scope cannot act"
    );
    let a = s.snapshot(&group("a"), 10).unwrap();
    assert_eq!((a.active, a.uncertain, a.queued), (0, 0, 0));
    assert_eq!(
        s.snapshot(&group("b"), 10).unwrap().cooldown_until,
        504,
        "cooldown survives reset"
    );
    let fresh = s.register_root(WorkloadClass::Interactive).unwrap();
    assert_eq!(fresh.epoch, 4);
    s.enqueue(fresh, 1, "a", 11).unwrap();
    assert!(
        s.next(&group("a"), 11).unwrap().is_some(),
        "new epoch grants"
    );
}

#[test]
fn withdraw_terminates_an_undelivered_grant_without_refunding_pacing() {
    let (mut s, one, two) = seeded();
    s.withdraw(one, 1, 1).unwrap();
    assert_eq!(
        s.status(one, 1, 1).unwrap(),
        RequestState::Terminal(TerminalOutcome::Cancelled)
    );
    assert_eq!(s.snapshot(&group("a"), 1).unwrap().active, 0);
    assert_eq!(
        s.next(&group("a"), 5).unwrap(),
        None,
        "request-start charge is retained"
    );
    assert_eq!(
        s.next(&group("a"), 10).unwrap(),
        Some(RequestId {
            scope: two,
            sequence: 1
        }),
        "round-robin moves to the other root once pacing releases"
    );
}

#[test]
fn withdraw_is_only_valid_for_active_attempts() {
    let (mut s, one, _two) = seeded();
    assert_eq!(s.withdraw(one, 2, 1), Err(AdmissionError::Conflict));
}

#[test]
fn next_wake_reports_the_earliest_state_change() {
    let (mut s, one, two) = seeded();
    assert_eq!(
        s.next_wake(1).unwrap(),
        Some(10),
        "pacing gate for queued work"
    );
    s.complete(one, 1, Feedback::Throttle { delay_ms: 200 }, 2)
        .unwrap();
    assert_eq!(
        s.next_wake(2).unwrap(),
        Some(202),
        "cooldown dominates pacing"
    );
    s.cancel(one, 2, 3).unwrap();
    s.cancel(two, 1, 3).unwrap();
    assert_eq!(s.next_wake(3).unwrap(), None, "nothing pending");
    s.enqueue(two, 2, "a", 300).unwrap();
    s.next(&group("a"), 300).unwrap().unwrap();
    assert_eq!(
        s.next_wake(300).unwrap(),
        Some(5_300),
        "active attempt deadline is the next change"
    );
    s.enqueue(two, 3, "a", 301).unwrap();
    assert_eq!(s.next_wake(301).unwrap(), Some(310));
    s.complete(two, 2, Feedback::Success, 302).unwrap();
    s.enqueue(one, 3, "b", 302).unwrap();
    s.next(&group("b"), 302).unwrap().unwrap();
    s.enqueue(one, 4, "b", 302).unwrap();
    assert_eq!(
        s.next_wake(302).unwrap(),
        Some(310),
        "capacity-blocked queue waits for its queue deadline or capacity, not pacing"
    );
}

#[test]
fn ledger_lists_outstanding_attempts_and_relative_group_deadlines() {
    let (mut s, one, two) = seeded();
    s.enqueue(two, 2, "b", 3).unwrap();
    s.next(&group("b"), 3).unwrap();
    s.complete(two, 2, Feedback::Throttle { delay_ms: 500 }, 4)
        .unwrap();
    let ledger = s.ledger(104).unwrap();
    assert_eq!(ledger.epoch, 3);
    assert_eq!(
        ledger
            .outstanding
            .iter()
            .map(|o| (o.group.clone(), o.scope, o.sequence))
            .collect::<Vec<_>>(),
        vec![(group("a"), one.serial, 1)]
    );
    assert_eq!(ledger.groups[&group("b")].cooldown_remaining_ms, 400);
    assert_eq!(ledger.groups[&group("a")].cooldown_remaining_ms, 0);
    assert_eq!(ledger.groups[&group("a")].pacing_remaining_ms, 0);
    assert_eq!(ledger.groups[&group("b")].pacing_remaining_ms, 0);
}

#[test]
fn restore_turns_outstanding_work_into_orphaned_quarantine_until_reset() {
    let (mut s, _one, _two) = seeded();
    let ledger = s.ledger(1).unwrap();
    let mut restored = AdmissionService::restore(config(), &ledger, 1_000).unwrap();
    let a = restored.snapshot(&group("a"), 1_000).unwrap();
    assert_eq!((a.active, a.uncertain), (1, 1), "orphan retains capacity");
    let root = restored.register_root(WorkloadClass::Interactive).unwrap();
    assert_eq!(root.epoch, 3, "same accounting epoch");
    restored.enqueue(root, 1, "a", 1_001).unwrap();
    assert_eq!(
        restored.next(&group("a"), 1_001),
        Err(AdmissionError::Quarantined)
    );
    assert_eq!(restored.ledger(1_001).unwrap().outstanding.len(), 1);
    assert_eq!(restored.reset(1_002).unwrap(), 4);
    assert_eq!(restored.snapshot(&group("a"), 1_002).unwrap().uncertain, 0);
}

#[test]
fn restore_rehydrates_cooldown_relative_to_the_new_clock() {
    let (mut s, _one, two) = seeded();
    s.enqueue(two, 2, "b", 3).unwrap();
    s.next(&group("b"), 3).unwrap();
    s.complete(two, 2, Feedback::Throttle { delay_ms: 500 }, 4)
        .unwrap();
    let ledger = s.ledger(104).unwrap();
    let mut restored = AdmissionService::restore(config(), &ledger, 50).unwrap();
    assert_eq!(
        restored.snapshot(&group("b"), 50).unwrap().cooldown_until,
        450
    );
    let ledger = restored.ledger(50).unwrap();
    assert_eq!(ledger.groups[&group("b")].cooldown_remaining_ms, 400);
}

#[test]
fn restore_rejects_a_ledger_for_an_unknown_group() {
    let (mut s, _one, _two) = seeded();
    let mut ledger = s.ledger(1).unwrap();
    ledger.outstanding[0].group = GroupId::new("zzz").unwrap();
    assert_eq!(
        AdmissionService::restore(config(), &ledger, 1).err(),
        Some(AdmissionError::UnknownGroup)
    );
}

#[test]
fn high_water_reports_the_last_accepted_sequence_per_scope() {
    let (mut s, one, two) = seeded();
    assert_eq!(s.high_water(one).unwrap(), 2);
    assert_eq!(s.high_water(two).unwrap(), 1);
    s.retire(two).unwrap();
    assert_eq!(s.high_water(two), Err(AdmissionError::UnknownScope));
}

/// Review MEDIUM-4: a sequence above the high-water mark was never enqueued,
/// so no grant can exist for it; refusing it must not poison any group.
#[test]
fn completing_a_never_enqueued_sequence_is_refused_without_poisoning_groups() {
    let (mut s, one, _two) = seeded();
    assert_eq!(
        s.complete(one, 99, Feedback::Success, 5),
        Err(AdmissionError::UnknownRequest)
    );
    assert!(!s.snapshot(&group("a"), 5).unwrap().unavailable);
    assert!(!s.snapshot(&group("b"), 5).unwrap().unavailable);
    s.enqueue(one, 3, "b", 6).unwrap();
    assert!(
        s.next(&group("b"), 6).unwrap().is_some(),
        "other group still grants"
    );
}

/// Review MEDIUM-5: the scope limit bounds live scopes; retired scopes free
/// their slot while serials are never recycled.
#[test]
fn scope_limit_counts_live_scopes_and_serials_are_never_recycled() {
    let mut cfg = config();
    cfg.max_scopes = 2;
    let mut s = AdmissionService::new(1, cfg).unwrap();
    let a = s.register_root(WorkloadClass::Interactive).unwrap();
    let b = s.register_root(WorkloadClass::Interactive).unwrap();
    assert_eq!(
        s.register_root(WorkloadClass::Interactive),
        Err(AdmissionError::ScopeLimit)
    );
    s.retire(a).unwrap();
    let c = s.register_root(WorkloadClass::Interactive).unwrap();
    assert!(c.serial > b.serial, "serial identity is never reused");
    assert_eq!(s.high_water(a), Err(AdmissionError::UnknownScope));
}
