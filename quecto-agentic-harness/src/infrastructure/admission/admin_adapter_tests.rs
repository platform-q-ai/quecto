use super::SocketAuthorityAdmin;
use crate::application::admission::dto::AuthorityAdminError;
use crate::application::admission::ports::AuthorityAdmin;

#[test]
fn no_authority_at_the_directory_is_not_running_for_inspect_and_reset_alike() {
    let dir = tempfile::TempDir::new().unwrap();
    let admin = SocketAuthorityAdmin::new();
    for (what, error) in [
        (
            "inspect",
            admin.inspect(dir.path()).map(|_| ()).unwrap_err(),
        ),
        ("reset", admin.reset(dir.path()).map(|_| ()).unwrap_err()),
    ] {
        match &error {
            AuthorityAdminError::NotRunning { directory, reason } => {
                assert_eq!(directory, dir.path(), "{what}");
                assert!(!reason.is_empty(), "{what}");
            }
            other => panic!("{what}: expected NotRunning, got {other:?}"),
        }
        assert!(
            error
                .to_string()
                .starts_with("no admission broker is running for directory"),
            "{what}: {error}"
        );
    }
    assert_eq!(format!("{admin:?}"), "SocketAuthorityAdmin");
}
