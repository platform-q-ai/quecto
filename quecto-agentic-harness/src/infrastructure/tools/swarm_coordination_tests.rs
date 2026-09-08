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
