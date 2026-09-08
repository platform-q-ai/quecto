//! AdmissionDispatcher reserve recovery regression (#1679 AC2).
use quecto::application::inference_admission::AdmissionService;
use quecto::application::ports::{AdmissionClient, AdmissionDispatcher, AdmissionRegistry};
use quecto::domain::inference_admission::{
    AdmissionConfig, Feedback, GroupId, GroupPolicy, WorkloadClass,
};
use std::collections::BTreeMap;

#[test]
fn long_interactive_attempt_cannot_starve_background_with_free_capacity() {
    let group = GroupId::new("g").unwrap();
    let mut s = AdmissionService::new(
        1,
        AdmissionConfig {
            groups: BTreeMap::from([(
                group.clone(),
                GroupPolicy {
                    capacity: 2,
                    reserve: 1,
                    min_interval_ms: 1,
                    queue_capacity: 10,
                    queue_timeout_ms: 100,
                    attempt_timeout_ms: 100,
                    fallback_base_ms: 100,
                    max_cooldown_ms: 100,
                },
            )]),
            aliases: BTreeMap::from([("g".into(), group.clone())]),
            max_scopes: 2,
            terminal_capacity: 10,
        },
    )
    .unwrap();
    let i = s.register_root(WorkloadClass::Interactive).unwrap();
    let b = s.register_child(i).unwrap();
    s.enqueue(i, 1, "g", 0).unwrap();
    s.next(&group, 0).unwrap().unwrap();
    s.enqueue(b, 1, "g", 1).unwrap();
    let mut observed = Vec::new();
    for now in 1..=4 {
        s.enqueue(i, now + 1, "g", now).unwrap();
        let grant = s.next(&group, now).unwrap().unwrap();
        observed.push(grant.scope);
        s.complete(grant.scope, grant.sequence, Feedback::Success, now)
            .unwrap();
    }
    assert!(
        observed.contains(&b),
        "eligible background must receive a shared opportunity"
    );
}

#[test]
fn short_interactive_attempts_cannot_starve_background_pacing_opportunities() {
    let group = GroupId::new("g").unwrap();
    let mut s = AdmissionService::new(
        1,
        AdmissionConfig {
            groups: BTreeMap::from([(
                group.clone(),
                GroupPolicy {
                    capacity: 2,
                    reserve: 1,
                    min_interval_ms: 1,
                    queue_capacity: 10,
                    queue_timeout_ms: 100,
                    attempt_timeout_ms: 100,
                    fallback_base_ms: 100,
                    max_cooldown_ms: 100,
                },
            )]),
            aliases: BTreeMap::from([("g".into(), group.clone())]),
            max_scopes: 2,
            terminal_capacity: 10,
        },
    )
    .unwrap();
    let i = s.register_root(WorkloadClass::Interactive).unwrap();
    let b = s.register_child(i).unwrap();
    s.enqueue(b, 1, "g", 0).unwrap();
    let mut observed = Vec::new();
    for now in 0..4 {
        s.enqueue(i, now + 1, "g", now).unwrap();
        let grant = s.next(&group, now).unwrap().unwrap();
        observed.push(grant.scope);
        s.complete(grant.scope, grant.sequence, Feedback::Success, now)
            .unwrap();
    }
    assert_eq!(observed, [i, i, i, b]);
}
