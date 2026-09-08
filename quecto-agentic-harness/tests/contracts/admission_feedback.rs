//! #1679 P2 AC5: receipt feedback is distinct from transport completion.
use quecto::application::inference_admission::AdmissionService;
use quecto::application::ports::{AdmissionClient, AdmissionDispatcher, AdmissionRegistry};
use quecto::domain::inference_admission::*;
use std::collections::BTreeMap;

fn setup() -> (AdmissionService, ScopeId, GroupId) {
    let group = GroupId::new("shared").unwrap();
    let other = GroupId::new("other").unwrap();
    let policy = GroupPolicy {
        capacity: 2,
        reserve: 0,
        min_interval_ms: 1,
        queue_capacity: 8,
        queue_timeout_ms: 1000,
        attempt_timeout_ms: 1000,
        fallback_base_ms: 100,
        max_cooldown_ms: 100,
    };
    let config = AdmissionConfig {
        groups: BTreeMap::from([(group.clone(), policy.clone()), (other.clone(), policy)]),
        aliases: BTreeMap::from([("account".into(), group.clone()), ("other".into(), other)]),
        max_scopes: 4,
        terminal_capacity: 4,
    };
    let mut service = AdmissionService::new(1, config).unwrap();
    let scope = service.register_root(WorkloadClass::Interactive).unwrap();
    service.enqueue(scope, 1, "account", 0).unwrap();
    service.next(&group, 0).unwrap().unwrap();
    (service, scope, group)
}

// Acceptance has its own tests. Effect tests intentionally do not unwrap receipt
// results: an unimplemented/rejected receipt must reach the effect's own oracle,
// rather than mask every Then with the same setup panic. This forwards to the real
// application port; it supplies no fake policy or successful receipt behavior.
fn receipt(
    service: &mut AdmissionService,
    scope: ScopeId,
    sequence: u64,
    report: u64,
    feedback: ThrottleFeedback,
    now: u64,
) -> Result<(), AdmissionError> {
    service.report_feedback(scope, sequence, report, feedback, now)
}

#[test]
fn active_receipt_is_accepted() {
    let (mut service, scope, _) = setup();
    assert_eq!(
        receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1),
        Ok(())
    );
}

#[test]
fn receipt_retains_transport_occupancy() {
    let (mut service, scope, group) = setup();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1);
    assert_eq!(service.snapshot(&group, 1).unwrap().active, 1);
}

#[test]
fn receipt_blocks_spare_capacity_before_deadline() {
    let (mut service, scope, group) = setup();
    service.enqueue(scope, 2, "account", 1).unwrap();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1);
    assert_eq!(service.next(&group, 89), Ok(None));
}

#[test]
fn receipt_allows_spare_capacity_at_exact_deadline() {
    let (mut service, scope, group) = setup();
    service.enqueue(scope, 2, "account", 1).unwrap();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1);
    assert_eq!(
        service.next(&group, 90),
        Ok(Some(RequestId { scope, sequence: 2 }))
    );
}

#[test]
fn late_completion_does_not_reanchor_receipt_deadline() {
    let (mut service, scope, group) = setup();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1);
    let _ = service.complete(scope, 1, Feedback::Failure, 50);
    assert_eq!(service.snapshot(&group, 50).unwrap().cooldown_until, 90);
}

#[test]
fn completion_releases_receipt_bearing_transport() {
    let (mut service, scope, group) = setup();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1);
    let _ = service.complete(scope, 1, Feedback::Failure, 50);
    assert_eq!(service.snapshot(&group, 50).unwrap().active, 0);
}

#[test]
fn duplicate_receipt_is_accepted() {
    let (mut service, scope, _) = setup();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1);
    assert_eq!(
        receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 2),
        Ok(())
    );
}

#[test]
fn duplicate_receipt_does_not_reanchor_deadline() {
    let (mut service, scope, group) = setup();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1);
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 2);
    assert_eq!(service.snapshot(&group, 2).unwrap().cooldown_until, 90);
}

#[test]
fn conflicting_receipt_is_rejected() {
    let (mut service, scope, _) = setup();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1);
    assert_eq!(
        receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(100), 2),
        Err(AdmissionError::Conflict)
    );
}

#[test]
fn conflicting_receipt_cannot_extend_deadline() {
    let (mut service, scope, group) = setup();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1);
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(100), 2);
    assert_eq!(service.snapshot(&group, 2).unwrap().cooldown_until, 90);
}

#[test]
fn shorter_receipt_does_not_shorten_deadline() {
    let (mut service, scope, group) = setup();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1);
    let _ = receipt(&mut service, scope, 1, 2, ThrottleFeedback::Until(20), 2);
    assert_eq!(service.snapshot(&group, 2).unwrap().cooldown_until, 90);
}

#[test]
fn unavailable_receipt_is_accepted() {
    let (mut service, scope, _) = setup();
    assert_eq!(
        receipt(&mut service, scope, 1, 1, ThrottleFeedback::Unavailable, 1),
        Ok(())
    );
}

#[test]
fn unavailable_receipt_marks_group_unavailable() {
    let (mut service, scope, group) = setup();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Unavailable, 1);
    assert!(service.snapshot(&group, 1).unwrap().unavailable);
}

#[test]
fn unavailable_receipt_retains_transport_occupancy() {
    let (mut service, scope, group) = setup();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Unavailable, 1);
    assert_eq!(service.snapshot(&group, 1).unwrap().active, 1);
}

#[test]
fn queued_receipt_is_rejected() {
    let (mut service, scope, _) = setup();
    service.enqueue(scope, 2, "account", 1).unwrap();
    assert_eq!(
        receipt(&mut service, scope, 2, 1, ThrottleFeedback::Until(90), 2),
        Err(AdmissionError::Conflict)
    );
}

#[test]
fn unknown_receipt_is_rejected() {
    let (mut service, scope, _) = setup();
    assert_eq!(
        receipt(&mut service, scope, 3, 1, ThrottleFeedback::Until(90), 2),
        Err(AdmissionError::UnknownRequest)
    );
}

#[test]
fn new_terminal_receipt_is_rejected() {
    let (mut service, scope, _) = setup();
    service.complete(scope, 1, Feedback::Success, 1).unwrap();
    assert_eq!(
        receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 2),
        Err(AdmissionError::Conflict)
    );
}

#[test]
fn queued_receipt_cannot_change_accounting() {
    let (mut service, scope, group) = setup();
    service.enqueue(scope, 2, "account", 1).unwrap();
    let _ = receipt(&mut service, scope, 2, 1, ThrottleFeedback::Until(90), 2);
    assert_eq!(
        service.snapshot(&group, 2).unwrap(),
        GroupSnapshot {
            active: 1,
            queued: 1,
            cooldown_until: 0,
            unavailable: false,
            observed_at: 2,
        }
    );
}

#[test]
fn unknown_receipt_cannot_change_accounting() {
    let (mut service, scope, group) = setup();
    let _ = receipt(&mut service, scope, 3, 1, ThrottleFeedback::Until(90), 2);
    assert_eq!(
        service.snapshot(&group, 2).unwrap(),
        GroupSnapshot {
            active: 1,
            queued: 0,
            cooldown_until: 0,
            unavailable: false,
            observed_at: 2,
        }
    );
}

#[test]
fn new_terminal_receipt_cannot_change_accounting() {
    let (mut service, scope, group) = setup();
    service.complete(scope, 1, Feedback::Success, 1).unwrap();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 2);
    assert_eq!(
        service.snapshot(&group, 2).unwrap(),
        GroupSnapshot {
            active: 0,
            queued: 0,
            cooldown_until: 0,
            unavailable: false,
            observed_at: 2,
        }
    );
}

fn siblings() -> (AdmissionService, ScopeId, GroupId) {
    let (mut service, scope, group) = setup();
    service.enqueue(scope, 2, "account", 1).unwrap();
    service.next(&group, 1).unwrap().unwrap();
    (service, scope, group)
}

#[test]
fn overlapping_siblings_merge_longer_deadline() {
    let (mut service, scope, group) = siblings();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(20), 1);
    let _ = receipt(&mut service, scope, 2, 1, ThrottleFeedback::Until(90), 2);
    assert_eq!(service.snapshot(&group, 2).unwrap().cooldown_until, 90);
}

#[test]
fn overlapping_siblings_keep_longer_deadline_in_reverse_order() {
    let (mut service, scope, group) = siblings();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1);
    let _ = receipt(&mut service, scope, 2, 1, ThrottleFeedback::Until(20), 2);
    assert_eq!(service.snapshot(&group, 2).unwrap().cooldown_until, 90);
}

#[test]
fn sibling_success_does_not_shorten_deadline() {
    let (mut service, scope, group) = siblings();
    let _ = receipt(&mut service, scope, 2, 1, ThrottleFeedback::Until(90), 2);
    let _ = service.complete(scope, 1, Feedback::Success, 3);
    assert_eq!(service.snapshot(&group, 3).unwrap().cooldown_until, 90);
}

#[test]
fn sibling_success_releases_only_completed_transport() {
    let (mut service, scope, group) = siblings();
    let _ = receipt(&mut service, scope, 2, 1, ThrottleFeedback::Until(90), 2);
    let _ = service.complete(scope, 1, Feedback::Success, 3);
    assert_eq!(service.snapshot(&group, 3).unwrap().active, 1);
}

#[test]
fn terminal_duplicate_receipt_is_accepted() {
    let (mut service, scope, _) = setup();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1);
    let _ = service.complete(scope, 1, Feedback::Failure, 3);
    assert_eq!(
        receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 4),
        Ok(())
    );
}

#[test]
fn terminal_duplicate_receipt_does_not_reapply_deadline() {
    let (mut service, scope, group) = setup();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1);
    let _ = service.complete(scope, 1, Feedback::Failure, 3);
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 4);
    assert_eq!(service.snapshot(&group, 4).unwrap().cooldown_until, 90);
}

#[test]
fn unavailable_denies_dispatch_before_completion() {
    let (mut service, scope, group) = setup();
    service.enqueue(scope, 2, "account", 1).unwrap();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Unavailable, 1);
    assert_eq!(service.next(&group, 2), Err(AdmissionError::Unavailable));
}

#[test]
fn unavailable_denies_dispatch_after_completion() {
    let (mut service, scope, group) = setup();
    service.enqueue(scope, 2, "account", 1).unwrap();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Unavailable, 1);
    let _ = service.complete(scope, 1, Feedback::Failure, 2);
    assert_eq!(service.next(&group, 200), Err(AdmissionError::Unavailable));
}

#[test]
fn unavailable_does_not_block_unrelated_group() {
    let (mut service, scope, _) = setup();
    let other = GroupId::new("other").unwrap();
    service.enqueue(scope, 2, "other", 1).unwrap();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Unavailable, 1);
    assert_eq!(
        service.next(&other, 2),
        Ok(Some(RequestId { scope, sequence: 2 }))
    );
}

#[test]
fn older_report_sequence_is_rejected_without_replay() {
    let (mut service, scope, _) = setup();
    let _ = receipt(&mut service, scope, 1, 2, ThrottleFeedback::Until(20), 1);
    assert_eq!(
        receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 2),
        Err(AdmissionError::Replay)
    );
}

#[test]
fn above_maximum_deadline_cannot_be_accepted_as_early_retry() {
    let (mut service, scope, group) = setup();
    let _ = receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(102), 1);
    assert!(service.snapshot(&group, 1).unwrap().unavailable);
}

#[test]
fn past_receipt_deadline_is_accepted_without_reanchoring() {
    let (mut service, scope, _) = setup();
    assert_eq!(
        receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(0), 1),
        Ok(())
    );
}

#[test]
fn cancelled_active_attempt_can_report_before_shutdown_ack() {
    let (mut service, scope, _) = setup();
    service.cancel(scope, 1, 1).unwrap();
    assert_eq!(
        receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1),
        Ok(())
    );
}

#[test]
fn regressed_receipt_clock_is_rejected() {
    let (mut service, scope, group) = setup();
    service.snapshot(&group, 2).unwrap();
    assert_eq!(
        receipt(&mut service, scope, 1, 1, ThrottleFeedback::Until(90), 1),
        Err(AdmissionError::TimeRegression)
    );
}
