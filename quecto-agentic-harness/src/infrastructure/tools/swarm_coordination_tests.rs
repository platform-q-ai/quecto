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
