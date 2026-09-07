//! Trusted AdmissionRegistry identity boundaries (#1679 AC3/AC6).
use quecto::application::inference_admission::AdmissionService;
use quecto::application::ports::{AdmissionClient, AdmissionRegistry};
use quecto::domain::inference_admission::{
    AdmissionConfig, AdmissionError, GroupId, GroupPolicy, ScopeId, WorkloadClass,
};
use std::collections::BTreeMap;

#[test]
fn unissued_and_retired_scope_handles_cannot_create_descendants() {
    let g = GroupId::new("g").unwrap();
    let mut service = AdmissionService::new(
        9,
        AdmissionConfig {
            groups: BTreeMap::from([(
                g.clone(),
                GroupPolicy {
                    capacity: 1,
                    reserve: 0,
                    min_interval_ms: 1,
                    queue_capacity: 1,
                    queue_timeout_ms: 1,
                    attempt_timeout_ms: 1,
                    max_cooldown_ms: 1,
                },
            )]),
            aliases: BTreeMap::from([("g".into(), g)]),
            max_scopes: 2,
            terminal_capacity: 1,
        },
    )
    .unwrap();
    let unknown = ScopeId {
        epoch: 9,
        serial: 99,
    };
    assert_eq!(
        service.register_child(unknown),
        Err(AdmissionError::UnknownScope)
    );
    assert_eq!(
        service.enqueue(unknown, 1, "g", 0),
        Err(AdmissionError::UnknownScope)
    );
    let root = service.register_root(WorkloadClass::Interactive).unwrap();
    service.retire(root).unwrap();
    assert_eq!(
        service.register_child(root),
        Err(AdmissionError::UnknownScope)
    );
}
