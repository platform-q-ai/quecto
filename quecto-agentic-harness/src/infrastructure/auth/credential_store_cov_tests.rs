use super::*;
use tempfile::TempDir;

fn credential(provider: &str, token: &str) -> Credential {
    Credential {
        provider: provider.into(),
        token: token.into(),
        method: AuthMethod::OAuth,
        expires_at: None,
        refresh_token: Some("refresh-secret".into()),
        account_id: Some("acct".into()),
    }
}

#[test]
fn debug_redacts_access_and_refresh_tokens() {
    let rendered = format!("{:?}", credential("openai", "sk-secret"));
    assert!(rendered.contains("[REDACTED]"));
    assert!(rendered.contains("openai"));
    assert!(!rendered.contains("sk-secret"));
    assert!(!rendered.contains("refresh-secret"));
}

#[test]
fn load_snapshot_missing_invalid_and_save_all_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path().join("nested"));
    assert!(store.load_snapshot().unwrap().is_empty());

    store.store(credential("anthropic", "tok-1")).unwrap();
    let loaded = store.load_snapshot().unwrap();
    assert_eq!(loaded["anthropic"].token, "tok-1");
    assert_eq!(
        loaded["anthropic"].refresh_token.as_deref(),
        Some("refresh-secret")
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(store.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    std::fs::write(store.path(), "{ definitely not json").unwrap();
    let err = store.load_snapshot().unwrap_err().to_string();
    assert!(err.contains("failed to parse credentials"));
}

#[test]
fn remove_missing_still_saves_empty_file() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    assert!(!store.remove("none").unwrap());
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert!(text.contains("credentials"));
}

#[test]
fn load_snapshot_reports_unreadable_credentials_file() {
    let dir = TempDir::new().expect("tempdir");
    // A directory where the credentials file is expected: readable as an entry,
    // but `read_to_string` fails with EISDIR.
    std::fs::create_dir(dir.path().join("credentials.json")).expect("create dir in place of file");

    let err = CredentialStore::new(dir.path())
        .load_snapshot()
        .expect_err("reading a directory as the credentials file must fail");

    assert!(
        err.to_string().contains("failed to read credentials"),
        "expected the read-stage message, got: {err}"
    );
}

#[test]
fn load_snapshot_reports_malformed_credentials_json() {
    let dir = TempDir::new().expect("tempdir");
    std::fs::write(dir.path().join("credentials.json"), "{ not json").expect("write bad json");

    let err = CredentialStore::new(dir.path())
        .load_snapshot()
        .expect_err("malformed JSON must fail");

    let msg = err.to_string();
    assert!(
        msg.contains("failed to parse credentials"),
        "expected the parse-stage message (distinct from the read stage), got: {msg}"
    );
    // The raw file contents must not be echoed back in the error.
    assert!(
        !msg.contains("not json"),
        "parse error leaked file contents: {msg}"
    );
}

#[test]
fn save_all_reports_credentials_dir_creation_failure() {
    let dir = TempDir::new().expect("tempdir");
    // Make the would-be parent directory a regular file so create_dir_all fails.
    let blocker = dir.path().join("blocked");
    std::fs::write(&blocker, b"not a directory").expect("write blocker file");

    let store = CredentialStore::new(blocker.join("nested"));
    let err = store
        .store(credential("openai", "sk-secret"))
        .expect_err("saving under a non-directory parent must fail");

    let msg = err.to_string();
    assert!(
        msg.contains("failed to create credentials dir"),
        "expected the dir-creation message, got: {msg}"
    );
    assert!(!msg.contains("sk-secret"), "error leaked the token: {msg}");
}

/// #2619 review: the store types its failure's kind where it happens, so a
/// caller never has to read it back out of the words.
#[test]
fn a_store_failure_is_typed_where_it_happens() {
    let kind = |store: &CredentialStore| CredentialStorePort::list(store).unwrap_err().failure;

    let malformed = TempDir::new().unwrap();
    std::fs::write(malformed.path().join("credentials.json"), "{ nope").unwrap();
    assert_eq!(
        kind(&CredentialStore::new(malformed.path())),
        CredentialStoreFailure::Malformed
    );

    let directory = TempDir::new().unwrap();
    std::fs::create_dir(directory.path().join("credentials.json")).unwrap();
    let store = CredentialStore::new(directory.path());
    assert_eq!(kind(&store), CredentialStoreFailure::Unavailable);
    let write = CredentialStorePort::store(&store, credential("openai", "fake-token"));
    assert_eq!(
        write.unwrap_err().failure,
        CredentialStoreFailure::Unavailable
    );
}

#[cfg(unix)]
#[test]
fn a_store_this_user_may_not_read_is_permission_denied() {
    use std::os::unix::fs::PermissionsExt;
    // SAFETY: geteuid has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        // root reads a 0o000 file: there is no refusal to observe.
        return;
    }
    let base = TempDir::new().unwrap();
    let path = base.path().join("credentials.json");
    std::fs::write(&path, r#"{"credentials":{}}"#).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let error = CredentialStorePort::list(&CredentialStore::new(base.path())).unwrap_err();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(error.failure, CredentialStoreFailure::PermissionDenied);
}
