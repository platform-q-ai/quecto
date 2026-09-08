//! #1679 P1: deterministic public-port admission contracts (AC1–3, AC6).
use quecto::application::inference_admission::AdmissionService;
use quecto::application::ports::{AdmissionClient, AdmissionDispatcher, AdmissionRegistry};
use quecto::domain::inference_admission::{
    AdmissionConfig, AdmissionError as Error, Feedback, GroupId, GroupPolicy, RequestId,
    RequestState, ScopeId, TerminalOutcome, WorkloadClass,
};
use std::collections::BTreeMap;

fn config(capacity: usize, reserve: usize) -> AdmissionConfig {
    let policy = GroupPolicy {
        capacity,
        reserve,
        min_interval_ms: 1,
        queue_capacity: 32,
        queue_timeout_ms: 100,
        attempt_timeout_ms: 100,
        fallback_base_ms: 1000,
        max_cooldown_ms: 1000,
    };
    AdmissionConfig {
        groups: BTreeMap::from([(group("a"), policy.clone()), (group("b"), policy)]),
        aliases: BTreeMap::from([("alpha".into(), group("a")), ("beta".into(), group("b"))]),
        max_scopes: 32,
        terminal_capacity: 8,
    }
}

fn group(name: &str) -> GroupId {
    GroupId::new(name).unwrap()
}

fn service(capacity: usize, reserve: usize) -> AdmissionService {
    AdmissionService::new(7, config(capacity, reserve)).unwrap()
}

fn root(s: &mut AdmissionService, class: WorkloadClass) -> ScopeId {
    s.register_root(class).unwrap()
}

fn enqueue(s: &mut AdmissionService, scope: ScopeId, sequence: u64, now: u64) -> RequestId {
    s.enqueue(scope, sequence, "alpha", now).unwrap();
    RequestId { scope, sequence }
}

fn grant(s: &mut AdmissionService, now: u64) -> RequestId {
    s.next(&group("a"), now).unwrap().unwrap()
}

#[test]
fn configuration_rejects_invalid_boundaries_and_incomplete_mapping() {
    let valid = config(1, 0);
    assert!(AdmissionService::new(7, valid.clone()).is_ok());
    let mut invalid = Vec::new();
    for (capacity, reserve) in [(0, 0), (1, 1), (2, 3)] {
        invalid.push(config(capacity, reserve));
    }
    for field in 0..5 {
        let mut c = valid.clone();
        let p = c.groups.get_mut(&group("a")).unwrap();
        match field {
            0 => p.min_interval_ms = 0,
            1 => p.queue_capacity = 0,
            2 => p.queue_timeout_ms = 0,
            3 => p.attempt_timeout_ms = 0,
            _ => p.max_cooldown_ms = 0,
        }
        invalid.push(c);
    }
    let mut c = valid.clone();
    c.aliases.insert("unknown".into(), group("missing"));
    invalid.push(c);
    let mut c = valid.clone();
    c.aliases.clear();
    invalid.push(c);
    let mut c = valid.clone();
    c.aliases.insert(String::new(), group("a"));
    invalid.push(c);
    let mut c = valid.clone();
    c.groups.clear();
    invalid.push(c);
    let mut c = valid.clone();
    c.max_scopes = 0;
    invalid.push(c);
    let mut c = valid;
    c.terminal_capacity = 0;
    invalid.push(c);
    for c in invalid {
        assert!(matches!(
            AdmissionService::new(7, c),
            Err(Error::InvalidConfig)
        ));
    }
    assert_eq!(GroupId::new(""), Err(Error::InvalidConfig));
}

#[test]
fn hierarchical_round_robin_fifo_does_not_reward_wide_roots() {
    let mut s = service(1, 0);
    let a = root(&mut s, WorkloadClass::Background);
    let a1 = s.register_child(a).unwrap();
    let a2 = s.register_child(a1).unwrap();
    let b = root(&mut s, WorkloadClass::Background);
    let a_first = enqueue(&mut s, a, 1, 0);
    let a_second = enqueue(&mut s, a, 2, 0);
    let child = enqueue(&mut s, a1, 1, 0);
    let grandchild = enqueue(&mut s, a2, 1, 0);
    let b_first = enqueue(&mut s, b, 1, 0);
    let b_second = enqueue(&mut s, b, 2, 0);
    let b_third = enqueue(&mut s, b, 3, 0);
    let expected = [
        a_first, b_first, child, b_second, grandchild, b_third, a_second,
    ];
    let mut actual = Vec::new();
    for now in 0..7 {
        let id = grant(&mut s, now);
        actual.push(id);
        s.complete(id.scope, id.sequence, Feedback::Success, now)
            .unwrap();
    }
    assert_eq!(actual, expected);
}

#[test]
fn shared_slots_use_three_to_one_and_fallback_is_work_conserving() {
    let mut s = service(1, 0);
    let interactive = root(&mut s, WorkloadClass::Interactive);
    let background = s.register_child(interactive).unwrap();
    for seq in 1..=8 {
        enqueue(&mut s, interactive, seq, 0);
    }
    for seq in 1..=2 {
        enqueue(&mut s, background, seq, 0);
    }
    let mut actual = Vec::new();
    for now in 0..10 {
        let id = grant(&mut s, now);
        actual.push(id.scope);
        s.complete(id.scope, id.sequence, Feedback::Success, now)
            .unwrap();
    }
    assert_eq!(
        actual,
        [
            interactive,
            interactive,
            interactive,
            background,
            interactive,
            interactive,
            interactive,
            background,
            interactive,
            interactive
        ]
    );
}

#[test]
fn reserve_is_inside_capacity_and_children_cannot_borrow_it() {
    let mut s = service(3, 1);
    let interactive = root(&mut s, WorkloadClass::Interactive);
    let child = s.register_child(interactive).unwrap();
    let grandchild = s.register_child(child).unwrap();
    let c = enqueue(&mut s, child, 1, 0);
    let g = enqueue(&mut s, grandchild, 1, 0);
    enqueue(&mut s, child, 2, 0);
    assert_eq!(grant(&mut s, 0), c);
    assert_eq!(grant(&mut s, 1), g);
    assert_eq!(s.next(&group("a"), 2), Ok(None));
    let i = enqueue(&mut s, interactive, 1, 2);
    assert_eq!(grant(&mut s, 2), i);
    assert_eq!(s.next(&group("a"), 3), Ok(None));
    assert_eq!(s.snapshot(&group("a"), 3).unwrap().active, 3);
}

#[test]
fn idle_parent_owns_no_slot_and_group_b_is_independent() {
    let mut s = service(1, 0);
    let parent = root(&mut s, WorkloadClass::Interactive);
    let child = s.register_child(parent).unwrap();
    let id = enqueue(&mut s, child, 1, 0);
    assert_eq!(grant(&mut s, 0), id);
    s.enqueue(parent, 1, "beta", 0).unwrap();
    assert_eq!(
        s.next(&group("b"), 0).unwrap(),
        Some(RequestId {
            scope: parent,
            sequence: 1
        })
    );
    s.complete(child, 1, Feedback::Success, 0).unwrap();
    assert_eq!(s.snapshot(&group("a"), 0).unwrap().active, 0);
    assert_eq!(s.snapshot(&group("b"), 0).unwrap().active, 1);
}

#[test]
fn queue_full_exact_deadline_and_cancel_never_dispatch() {
    let mut c = config(1, 0);
    c.groups.get_mut(&group("a")).unwrap().queue_capacity = 1;
    let mut s = AdmissionService::new(7, c).unwrap();
    let a = root(&mut s, WorkloadClass::Background);
    enqueue(&mut s, a, 1, 0);
    assert_eq!(s.enqueue(a, 2, "alpha", 0), Err(Error::QueueFull));
    assert_eq!(
        s.status(a, 1, 99).unwrap(),
        RequestState::Queued { deadline: 100 }
    );
    assert_eq!(s.next(&group("a"), 100), Ok(None));
    assert_eq!(
        s.status(a, 1, 100).unwrap(),
        RequestState::Terminal(TerminalOutcome::TimedOut)
    );
    assert_eq!(
        s.enqueue(a, 1, "alpha", 100).unwrap(),
        RequestState::Terminal(TerminalOutcome::TimedOut)
    );
    enqueue(&mut s, a, 2, 100);
    assert_eq!(
        s.cancel(a, 2, 100).unwrap(),
        RequestState::Terminal(TerminalOutcome::Cancelled)
    );
    assert_eq!(s.next(&group("a"), 101), Ok(None));
    assert_eq!(s.snapshot(&group("a"), 101).unwrap().queued, 0);
}

#[test]
fn active_cancel_and_attempt_deadline_require_confirmed_completion() {
    let mut s = service(1, 0);
    let a = root(&mut s, WorkloadClass::Background);
    enqueue(&mut s, a, 1, 0);
    grant(&mut s, 0);
    enqueue(&mut s, a, 2, 99);
    assert_eq!(
        s.status(a, 1, 100).unwrap(),
        RequestState::Active {
            started: 0,
            deadline: 100,
            cancellation_required: true,
        }
    );
    s.cancel(a, 1, 100).unwrap();
    assert_eq!(s.next(&group("a"), 100), Ok(None));
    assert_eq!(s.snapshot(&group("a"), 100).unwrap().active, 1);
    s.complete(a, 1, Feedback::Success, 100).unwrap();
    assert_eq!(
        grant(&mut s, 100),
        RequestId {
            scope: a,
            sequence: 2
        }
    );
    s.complete(a, 1, Feedback::Success, 100).unwrap();
    assert_eq!(s.snapshot(&group("a"), 100).unwrap().active, 1);
}

#[test]
fn pacing_is_not_refunded_and_cooldown_only_extends() {
    let mut c = config(2, 0);
    c.groups.get_mut(&group("a")).unwrap().min_interval_ms = 10;
    let mut s = AdmissionService::new(7, c).unwrap();
    let a = root(&mut s, WorkloadClass::Background);
    enqueue(&mut s, a, 1, 0);
    enqueue(&mut s, a, 2, 0);
    grant(&mut s, 0);
    s.complete(a, 1, Feedback::Throttle { delay_ms: 20 }, 0)
        .unwrap();
    assert_eq!(s.next(&group("a"), 19), Ok(None));
    assert_eq!(grant(&mut s, 20).sequence, 2);
    s.complete(a, 2, Feedback::Throttle { delay_ms: 0 }, 20)
        .unwrap();
    enqueue(&mut s, a, 3, 20);
    assert_eq!(s.next(&group("a"), 29), Ok(None));
    assert_eq!(grant(&mut s, 30).sequence, 3);
    assert_eq!(s.next(&group("a"), 29), Err(Error::TimeRegression));
}

#[test]
fn duplicate_and_conflicting_acquire_terminal_eviction_and_scope_limits() {
    let mut c = config(1, 0);
    c.terminal_capacity = 1;
    c.max_scopes = 2;
    let mut s = AdmissionService::new(7, c).unwrap();
    let a = root(&mut s, WorkloadClass::Interactive);
    let b = s.register_child(a).unwrap();
    assert_eq!(s.register_child(b), Err(Error::ScopeLimit));
    enqueue(&mut s, a, 1, 0);
    assert_eq!(
        s.enqueue(a, 1, "alpha", 0).unwrap(),
        RequestState::Queued { deadline: 100 }
    );
    assert_eq!(s.enqueue(a, 1, "beta", 0), Err(Error::Conflict));
    grant(&mut s, 0);
    assert_eq!(
        s.enqueue(a, 1, "alpha", 0).unwrap(),
        RequestState::Active {
            started: 0,
            deadline: 100,
            cancellation_required: false,
        }
    );
    s.complete(a, 1, Feedback::Success, 0).unwrap();
    enqueue(&mut s, a, 2, 1);
    grant(&mut s, 1);
    s.complete(a, 2, Feedback::Success, 1).unwrap();
    assert_eq!(s.enqueue(a, 1, "alpha", 1), Err(Error::Replay));
    assert_eq!(s.complete(a, 1, Feedback::Success, 1), Err(Error::Replay));
    assert!(!s.snapshot(&group("a"), 1).unwrap().unavailable);
    assert_eq!(
        s.register_root(WorkloadClass::Interactive),
        Err(Error::ScopeLimit),
        "the limit bounds live scopes"
    );
    s.retire(b).unwrap();
    assert_eq!(s.enqueue(b, 1, "alpha", 1), Err(Error::UnknownScope));
    // Retirement frees the slot; the burned serial is never reused (P3 review).
    let c = s.register_root(WorkloadClass::Interactive).unwrap();
    assert!(c.serial > b.serial);
}

#[test]
fn unknown_identity_and_completion_do_not_release_other_attempts() {
    let mut s = service(1, 0);
    let a = root(&mut s, WorkloadClass::Background);
    let b = root(&mut s, WorkloadClass::Background);
    enqueue(&mut s, a, 1, 0);
    grant(&mut s, 0);
    assert_eq!(s.cancel(b, 1, 0), Err(Error::UnknownRequest));
    assert_eq!(s.enqueue(a, 2, "missing", 0), Err(Error::UnknownGroup));
    let stale = ScopeId {
        epoch: 6,
        serial: a.serial,
    };
    assert_eq!(
        s.complete(stale, 1, Feedback::Success, 0),
        Err(Error::StaleEpoch)
    );
    assert_eq!(s.snapshot(&group("a"), 0).unwrap().active, 1);
    // A sequence above the high-water mark was never enqueued, so no grant can
    // exist for it: refused, and no group is poisoned (P3 review MEDIUM-4).
    assert_eq!(
        s.complete(a, 99, Feedback::Success, 0),
        Err(Error::UnknownRequest)
    );
    assert!(!s.snapshot(&group("a"), 0).unwrap().unavailable);
    assert_eq!(s.snapshot(&group("a"), 0).unwrap().active, 1);
}

#[test]
fn cooldown_excess_and_arithmetic_overflow_fail_closed() {
    for delay in [1000, 1001, u64::MAX] {
        let mut s = service(1, 0);
        let a = root(&mut s, WorkloadClass::Background);
        enqueue(&mut s, a, 1, 0);
        grant(&mut s, 0);
        s.complete(a, 1, Feedback::Throttle { delay_ms: delay }, 0)
            .unwrap();
        assert_eq!(
            s.snapshot(&group("a"), 0).unwrap().unavailable,
            delay > 1000
        );
        s.enqueue(a, 2, "beta", 0).unwrap();
        assert!(s.next(&group("b"), 0).unwrap().is_some());
    }
    let mut s = service(1, 0);
    let a = root(&mut s, WorkloadClass::Background);
    assert_eq!(s.enqueue(a, 1, "alpha", u64::MAX), Err(Error::Unavailable));
    assert!(s.snapshot(&group("a"), u64::MAX).unwrap().unavailable);
}

#[test]
fn completion_feedback_is_once_even_after_later_success() {
    let mut s = service(2, 0);
    let a = root(&mut s, WorkloadClass::Background);
    enqueue(&mut s, a, 1, 0);
    enqueue(&mut s, a, 2, 0);
    grant(&mut s, 0);
    grant(&mut s, 1);
    s.complete(a, 1, Feedback::Throttle { delay_ms: 20 }, 1)
        .unwrap();
    s.complete(a, 2, Feedback::Success, 2).unwrap();
    s.complete(a, 1, Feedback::Throttle { delay_ms: 20 }, 10)
        .unwrap();
    assert_eq!(s.snapshot(&group("a"), 10).unwrap().cooldown_until, 21);
    assert_eq!(
        s.complete(a, 1, Feedback::Success, 10),
        Err(Error::Conflict)
    );
    enqueue(&mut s, a, 3, 10);
    assert_eq!(s.next(&group("a"), 20), Ok(None));
    assert_eq!(grant(&mut s, 21).sequence, 3);
}

#[test]
fn active_cancellation_before_deadline_and_retirement_preserve_capacity() {
    let mut s = service(1, 0);
    let a = root(&mut s, WorkloadClass::Background);
    let b = s.register_child(a).unwrap();
    enqueue(&mut s, a, 1, 0);
    grant(&mut s, 0);
    enqueue(&mut s, b, 1, 0);
    assert_eq!(
        s.status(a, 1, 1).unwrap(),
        RequestState::Active {
            started: 0,
            deadline: 100,
            cancellation_required: false,
        }
    );
    assert_eq!(
        s.cancel(a, 1, 1).unwrap(),
        RequestState::Active {
            started: 0,
            deadline: 100,
            cancellation_required: true,
        }
    );
    assert_eq!(s.next(&group("a"), 1), Ok(None));
    assert_eq!(s.retire(a), Err(Error::Busy));
    s.retire(b).unwrap();
    assert_eq!(s.snapshot(&group("a"), 1).unwrap().queued, 0);
    s.complete(a, 1, Feedback::Success, 1).unwrap();
    assert_eq!(s.snapshot(&group("a"), 1).unwrap().active, 0);
}

#[test]
fn shorter_feedback_during_cooldown_cannot_shorten_it() {
    let mut s = service(3, 0);
    let a = root(&mut s, WorkloadClass::Background);
    for seq in 1..=3 {
        enqueue(&mut s, a, seq, 0);
    }
    for now in 0..3 {
        grant(&mut s, now);
    }
    s.complete(a, 1, Feedback::Throttle { delay_ms: 1000 }, 2)
        .unwrap();
    s.complete(a, 2, Feedback::Throttle { delay_ms: 10 }, 3)
        .unwrap();
    s.complete(a, 3, Feedback::Throttle { delay_ms: 0 }, 4)
        .unwrap();
    assert_eq!(s.snapshot(&group("a"), 4).unwrap().cooldown_until, 1002);
    enqueue(&mut s, a, 4, 1000);
    assert_eq!(s.next(&group("a"), 1001), Ok(None));
    assert_eq!(grant(&mut s, 1002).sequence, 4);
}

#[test]
fn reserved_grants_do_not_consume_shared_arbitration_turns() {
    let mut s = service(2, 1);
    let i = root(&mut s, WorkloadClass::Interactive);
    let b = s.register_child(i).unwrap();
    for seq in 1..=8 {
        enqueue(&mut s, i, seq, 0);
    }
    enqueue(&mut s, b, 1, 0);
    let mut shared = Vec::new();
    for round in 0..4 {
        let now = round * 2;
        let first = grant(&mut s, now);
        shared.push(first.scope);
        let reserved = grant(&mut s, now + 1);
        assert_eq!(reserved.scope, i);
        s.complete(first.scope, first.sequence, Feedback::Success, now + 1)
            .unwrap();
        s.complete(
            reserved.scope,
            reserved.sequence,
            Feedback::Success,
            now + 1,
        )
        .unwrap();
    }
    assert_eq!(shared, [i, i, i, b]);
}

#[test]
fn regressions_and_unknown_completion_fail_without_unsafe_dispatch() {
    let mut s = service(1, 0);
    let a = root(&mut s, WorkloadClass::Background);
    enqueue(&mut s, a, 2, 10);
    assert_eq!(s.enqueue(a, 1, "alpha", 10), Err(Error::Replay));
    assert_eq!(s.enqueue(a, 3, "alpha", 9), Err(Error::TimeRegression));
    grant(&mut s, 10);
    assert_eq!(
        s.complete(a, 2, Feedback::Throttle { delay_ms: 20 }, 9),
        Err(Error::TimeRegression)
    );
    assert_eq!(s.snapshot(&group("a"), 10).unwrap().active, 1);
    assert_eq!(s.snapshot(&group("a"), 10).unwrap().cooldown_until, 0);
    assert_eq!(s.enqueue(a, 2, "beta", 10), Err(Error::Conflict));
    enqueue(&mut s, a, 3, 10);
    assert_eq!(
        s.complete(a, 99, Feedback::Success, 10),
        Err(Error::UnknownRequest)
    );
    s.complete(a, 2, Feedback::Success, 10).unwrap();
    assert_eq!(
        s.next(&group("a"), 11).unwrap(),
        Some(RequestId {
            scope: a,
            sequence: 3
        }),
        "a never-enqueued completion does not poison dispatch"
    );
    assert_eq!(s.enqueue(a, 2, "beta", 11), Err(Error::Conflict));
}

#[test]
fn accepted_cooldown_and_dispatch_deadlines_cannot_wrap_time() {
    let mut c = config(1, 0);
    let p = c.groups.get_mut(&group("a")).unwrap();
    p.queue_timeout_ms = 1;
    p.attempt_timeout_ms = 1;
    p.min_interval_ms = 1;
    let mut s = AdmissionService::new(7, c).unwrap();
    let a = root(&mut s, WorkloadClass::Background);
    enqueue(&mut s, a, 1, u64::MAX - 10);
    grant(&mut s, u64::MAX - 10);
    s.complete(a, 1, Feedback::Throttle { delay_ms: 20 }, u64::MAX - 10)
        .unwrap();
    assert!(s.snapshot(&group("a"), u64::MAX - 10).unwrap().unavailable);
    assert_eq!(s.snapshot(&group("a"), u64::MAX - 10).unwrap().active, 0);
    for pacing_overflow in [false, true] {
        let mut c = config(1, 0);
        let p = c.groups.get_mut(&group("a")).unwrap();
        p.queue_timeout_ms = 1;
        p.attempt_timeout_ms = if pacing_overflow { 1 } else { 100 };
        p.min_interval_ms = if pacing_overflow { 100 } else { 1 };
        let mut s = AdmissionService::new(7, c).unwrap();
        let a = root(&mut s, WorkloadClass::Background);
        enqueue(&mut s, a, 1, u64::MAX - 10);
        assert_eq!(s.next(&group("a"), u64::MAX - 10), Err(Error::Unavailable));
        assert_eq!(s.snapshot(&group("a"), u64::MAX - 10).unwrap().active, 0);
    }
}

#[test]
fn saturated_group_queue_and_evicted_completion_do_not_affect_live_work() {
    let mut c = config(1, 0);
    c.groups.get_mut(&group("a")).unwrap().queue_capacity = 1;
    c.terminal_capacity = 1;
    let mut s = AdmissionService::new(7, c).unwrap();
    let a = root(&mut s, WorkloadClass::Background);
    enqueue(&mut s, a, 1, 0);
    assert_eq!(s.enqueue(a, 2, "alpha", 0), Err(Error::QueueFull));
    s.enqueue(a, 2, "beta", 0).unwrap();
    assert!(s.next(&group("b"), 0).unwrap().is_some());
    grant(&mut s, 0);
    s.complete(a, 1, Feedback::Success, 0).unwrap();
    s.complete(a, 2, Feedback::Success, 0).unwrap();
    enqueue(&mut s, a, 3, 1);
    grant(&mut s, 1);
    assert_eq!(s.complete(a, 1, Feedback::Success, 1), Err(Error::Replay));
    assert_eq!(s.snapshot(&group("a"), 1).unwrap().active, 1);
}

#[test]
fn drained_roots_and_agents_rejoin_without_resetting_peers_turns() {
    let mut s = service(1, 0);
    let a = root(&mut s, WorkloadClass::Background);
    let child = s.register_child(a).unwrap();
    let b = root(&mut s, WorkloadClass::Background);
    enqueue(&mut s, a, 1, 0);
    enqueue(&mut s, b, 1, 0);
    let first = grant(&mut s, 0);
    s.complete(first.scope, first.sequence, Feedback::Success, 0)
        .unwrap();
    enqueue(&mut s, child, 1, 1);
    enqueue(&mut s, a, 2, 1);
    let second = grant(&mut s, 1);
    s.complete(second.scope, second.sequence, Feedback::Success, 1)
        .unwrap();
    let third = grant(&mut s, 2);
    s.complete(third.scope, third.sequence, Feedback::Success, 2)
        .unwrap();
    enqueue(&mut s, b, 2, 3);
    let fourth = grant(&mut s, 3);
    assert_eq!(
        [first.scope, second.scope, third.scope, fourth.scope],
        [a, b, child, b]
    );
}
