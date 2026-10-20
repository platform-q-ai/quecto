use super::*;

#[test]
fn a_store_failure_maps_by_its_kind() {
    assert_eq!(
        unavailable(CredentialStoreFailure::Malformed),
        StoredCredentialsUnavailable::Malformed
    );
    assert_eq!(
        unavailable(CredentialStoreFailure::PermissionDenied),
        StoredCredentialsUnavailable::PermissionDenied
    );
    assert_eq!(
        unavailable(CredentialStoreFailure::Unavailable),
        StoredCredentialsUnavailable::Unreadable
    );
}
