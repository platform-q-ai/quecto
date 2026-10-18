//! #2192 review round 5: a launch whose uuid a child already reported (a
//! guessed one) replaces the reported row — this harness's own launch is
//! authoritative — while any other duplicate key is still refused.
use super::*;
use crate::domain::ids::AgentUuid;

fn row(uuid: &str, origin: ChildOrigin) -> SubagentEntry {
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new(uuid),
        "row".into(),
        "/tmp/row.sock".into(),
        0,
    );
    entry.origin = origin;
    entry
}

fn accepting() -> SharedHarnessLifecycle {
    super::super::harness_lifecycle::new_shared_harness_lifecycle()
}

#[test]
fn a_launch_replaces_a_row_a_child_reported_under_its_uuid() {
    let registry = super::super::subagent_registry::new_registry();
    let uuid = "65268567-be4a-471f-a805-1238dcf08b68";
    let mut reported = row(uuid, ChildOrigin::Reported);
    reported.parent_id = Some("the-reporter".into());
    registry.lock().unwrap().insert(uuid.into(), reported);
    register_and_broadcast(
        &registry,
        None,
        "mine",
        row(uuid, ChildOrigin::Launched),
        &accepting(),
    )
    .expect("the launch takes its key back");
    let guard = registry.lock().unwrap();
    assert_eq!(guard[uuid].origin, ChildOrigin::Launched);
    assert_eq!(guard[uuid].display_name, "mine");
    assert_eq!(guard[uuid].parent_id, None);
}

#[test]
fn any_other_duplicate_key_is_still_refused() {
    let uuid = "65268567-be4a-471f-a805-1238dcf08b68";
    for (existing, incoming) in [
        (ChildOrigin::Launched, ChildOrigin::Launched),
        (ChildOrigin::Unverified, ChildOrigin::Launched),
        (ChildOrigin::Reported, ChildOrigin::Unverified),
    ] {
        let registry = super::super::subagent_registry::new_registry();
        registry
            .lock()
            .unwrap()
            .insert(uuid.into(), row(uuid, existing));
        let refused =
            register_and_broadcast(&registry, None, "mine", row(uuid, incoming), &accepting());
        assert!(refused.is_err(), "{existing:?} then {incoming:?}");
        assert_eq!(registry.lock().unwrap()[uuid].origin, existing);
    }
}
