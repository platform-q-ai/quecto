use super::*;
#[test]
fn malformed_wire_membership_is_rejected_instead_of_silently_skipping_cleanup() {
    for members in [
        json!(null),
        json!([{ "id":"p", "status":"live", "pid":123 }]),
        json!([{ "id":"p", "status":"bogus" }]),
    ] {
        assert!(
            decode(json!({"status":"failed","coordinator":"p","deadline":1,"members":members}))
                .is_err()
        );
    }
    assert!(
        decode(json!({"status":"unknown","coordinator":"p","deadline":1,"members":[]})).is_err()
    );
}
