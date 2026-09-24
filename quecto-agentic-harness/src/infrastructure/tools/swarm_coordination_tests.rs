use super::*;
#[test]
fn malformed_wire_membership_is_rejected_instead_of_silently_skipping_cleanup() {
    for members in [
        json!(null),
        json!([{ "id":"p", "status":"live", "pid":123 }]),
        json!([{ "id":"p", "status":"bogus" }]),
    ] {
        assert!(
            decode(json!({"control_generation":0,"status":"failed","coordinator":"p","deadline":1,"members":members}))
                .is_err()
        );
    }
    assert!(
        decode(json!({"control_generation":0,"status":"unknown","coordinator":"p","deadline":1,"members":[]})).is_err()
    );
}

#[test]
fn paused_snapshot_is_readable_and_does_not_request_terminal_cleanup() {
    let snapshot = decode(
        json!({"control_generation":0,"status":"paused","coordinator":"p","deadline":1,
        "members":[{"id":"p","status":"live"}]}),
    )
    .expect("a paused run must remain inspectable");
    assert!(!snapshot.status.terminal());
}

#[test]
fn control_receipts_require_typed_status_generation_and_budget() {
    for value in [
        json!({}),
        json!({"status":"running"}),
        json!({"status":"unknown","generation":1}),
        json!({"status":"running","generation":-1}),
        json!({"status":"running","generation":1,"budget":{}}),
    ] {
        assert!(SwarmContext::decode_control_receipt(value, false).is_err());
    }
    let receipt =
        SwarmContext::decode_control_receipt(json!({"status":"paused","generation":7}), false)
            .unwrap();
    assert_eq!(receipt.status, RunStatus::Paused);
    assert_eq!(receipt.generation, 7);
}

#[test]
fn each_member_carries_its_launcher_so_settlement_knows_who_ends_it() {
    // #2121: the store records who launched each member; a legacy row or the
    // bootstrapped coordinator has none.
    let snapshot = decode(
        json!({"control_generation":0,"status":"succeeded","coordinator":"p",
        "deadline":1,"members":[
            {"id":"p","status":"live","launcher":null},
            {"id":"w","status":"live","launcher":"p"},
            {"id":"legacy","status":"live"}
        ]}),
    )
    .expect("a closed run is readable");
    let launchers: Vec<_> = snapshot
        .members
        .iter()
        .map(|m| m.launcher.as_deref())
        .collect();
    assert_eq!(launchers, [None, Some("p"), None]);
}

#[test]
fn a_relative_deadline_becomes_now_plus_the_seconds_and_an_absolute_one_wins() {
    // #2125: a model need not know the current Unix time to create a run.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let relative = absolute_deadline(&json!({"deadline_in_seconds": 3600}));
    let at = relative.as_f64().expect("a number");
    assert!((now + 3590.0..now + 3610.0).contains(&at), "{at}");
    assert_eq!(
        absolute_deadline(&json!({"deadline": 42, "deadline_in_seconds": 3600})),
        json!(42)
    );
    assert!(
        absolute_deadline(&json!({})).is_null(),
        "the store refuses a missing deadline"
    );
}
