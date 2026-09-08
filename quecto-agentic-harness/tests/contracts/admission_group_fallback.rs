//! Shared no-hint fallback belongs to group state, not leaf retry decorators.
use quecto::application::inference_admission::AdmissionService;
use quecto::application::ports::{AdmissionClient, AdmissionDispatcher, AdmissionRegistry};
use quecto::domain::inference_admission::*;
use std::collections::BTreeMap;
fn setup() -> (AdmissionService, ScopeId, GroupId) {
    let group = GroupId::new("shared").unwrap();
    let mut service = AdmissionService::new(
        1,
        AdmissionConfig {
            groups: BTreeMap::from([(
                group.clone(),
                GroupPolicy {
                    capacity: 2,
                    reserve: 0,
                    min_interval_ms: 1,
                    queue_capacity: 4,
                    queue_timeout_ms: 100000,
                    attempt_timeout_ms: 100000,
                    fallback_base_ms: 1000,
                    max_cooldown_ms: 10000,
                },
            )]),
            aliases: BTreeMap::from([("a".into(), group.clone()), ("b".into(), group.clone())]),
            max_scopes: 2,
            terminal_capacity: 4,
        },
    )
    .unwrap();
    let scope = service.register_root(WorkloadClass::Interactive).unwrap();
    for (sequence, alias) in [(1, "a"), (2, "b")] {
        service.enqueue(scope, sequence, alias, sequence).unwrap();
        service.next(&group, sequence).unwrap();
    }
    (service, scope, group)
}
#[test]
fn sibling_no_hint_reports_advance_one_group_counter() {
    let (mut service, scope, group) = setup();
    let _ = service.report_feedback(
        scope,
        1,
        1,
        ThrottleFeedback::NoHint { jitter: u64::MAX },
        2,
    );
    let _ = service.report_feedback(
        scope,
        2,
        1,
        ThrottleFeedback::NoHint { jitter: u64::MAX },
        2,
    );
    assert_eq!(service.snapshot(&group, 2).unwrap().cooldown_until, 2002);
}
#[test]
fn duplicate_no_hint_report_does_not_advance_next_fallback() {
    let (mut service, scope, group) = setup();
    for _ in 0..2 {
        let _ = service.report_feedback(
            scope,
            1,
            1,
            ThrottleFeedback::NoHint { jitter: u64::MAX },
            2,
        );
    }
    let _ = service.report_feedback(
        scope,
        2,
        1,
        ThrottleFeedback::NoHint { jitter: u64::MAX },
        2,
    );
    assert_eq!(service.snapshot(&group, 2).unwrap().cooldown_until, 2002);
}

#[test]
fn successful_sibling_resets_shared_fallback_counter() {
    let (mut service, scope, group) = setup();
    let _ = service.report_feedback(
        scope,
        1,
        1,
        ThrottleFeedback::NoHint { jitter: u64::MAX },
        2,
    );
    let _ = service.complete(scope, 2, Feedback::Success, 2);
    let _ = service.report_feedback(
        scope,
        1,
        2,
        ThrottleFeedback::NoHint { jitter: u64::MAX },
        1002,
    );
    assert_eq!(service.snapshot(&group, 1002).unwrap().cooldown_until, 2002);
}

fn configured_service(base: u64, maximum: u64) -> Result<AdmissionService, AdmissionError> {
    let group = GroupId::new("configured").unwrap();
    AdmissionService::new(
        1,
        AdmissionConfig {
            groups: BTreeMap::from([(
                group.clone(),
                GroupPolicy {
                    capacity: 2,
                    reserve: 0,
                    min_interval_ms: 1,
                    queue_capacity: 4,
                    queue_timeout_ms: 100_000,
                    attempt_timeout_ms: 100_000,
                    fallback_base_ms: base,
                    max_cooldown_ms: maximum,
                },
            )]),
            aliases: BTreeMap::from([("configured".into(), group)]),
            max_scopes: 2,
            terminal_capacity: 4,
        },
    )
}

#[test]
fn operator_configured_ten_second_base_controls_initial_receipt_delay() {
    let mut service = configured_service(10_000, 80_000).unwrap();
    let group = GroupId::new("configured").unwrap();
    let scope = service.register_root(WorkloadClass::Interactive).unwrap();
    service.enqueue(scope, 1, "configured", 0).unwrap();
    assert_eq!(
        service.next(&group, 0).unwrap(),
        Some(RequestId { scope, sequence: 1 })
    );
    service
        .report_feedback(
            scope,
            1,
            1,
            ThrottleFeedback::NoHint { jitter: u64::MAX },
            2,
        )
        .unwrap();
    assert_eq!(service.snapshot(&group, 2).unwrap().cooldown_until, 10_002);
}

#[test]
fn operator_configured_zero_base_is_rejected_before_admission() {
    assert!(matches!(
        configured_service(0, 80_000),
        Err(AdmissionError::InvalidConfig)
    ));
}

#[test]
fn operator_configured_base_above_maximum_is_rejected_before_admission() {
    assert!(matches!(
        configured_service(80_001, 80_000),
        Err(AdmissionError::InvalidConfig)
    ));
}
