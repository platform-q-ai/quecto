use super::*;

#[test]
fn a_store_reason_maps_to_its_kind_only() {
    assert_eq!(
        unavailable("config error: failed to parse credentials: a field has the wrong type"),
        StoredCredentialsUnavailable::Malformed
    );
    assert_eq!(
        unavailable("config error: failed to read credentials: Permission denied (os error 13)"),
        StoredCredentialsUnavailable::PermissionDenied
    );
    assert_eq!(
        unavailable("config error: failed to read credentials: Is a directory (os error 21)"),
        StoredCredentialsUnavailable::Unreadable
    );
}
