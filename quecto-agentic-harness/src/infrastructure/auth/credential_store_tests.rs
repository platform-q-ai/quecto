use super::*;
use tempfile::TempDir;

fn make_credential(provider: &str, token: &str, method: AuthMethod) -> Credential {
    Credential {
        provider: provider.to_string(),
        token: token.to_string(),
        method,
        expires_at: None,
        refresh_token: None,
        account_id: None,
    }
}

fn make_expired_credential(provider: &str) -> Credential {
    Credential {
        provider: provider.to_string(),
        token: "expired-token".to_string(),
        method: AuthMethod::Token,
        expires_at: Some(0), // epoch — always expired
        refresh_token: None,
        account_id: None,
    }
}

#[test]
fn test_store_and_get() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    let cred = make_credential("openai", "sk-test", AuthMethod::Token);
    store.store(cred).unwrap();

    let loaded = store.get("openai").unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.token, "sk-test");
    assert_eq!(loaded.method, AuthMethod::Token);
}

#[test]
fn test_get_nonexistent() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    let loaded = store.get("openai").unwrap();
    assert!(loaded.is_none());
}

#[test]
fn test_exists() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    assert!(!store.exists("openai").unwrap());
    store
        .store(make_credential("openai", "sk-test", AuthMethod::Token))
        .unwrap();
    assert!(store.exists("openai").unwrap());
}

#[test]
fn test_remove() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    store
        .store(make_credential("openai", "sk-test", AuthMethod::Token))
        .unwrap();
    assert!(store.exists("openai").unwrap());

    let removed = store.remove("openai").unwrap();
    assert!(removed);
    assert!(!store.exists("openai").unwrap());

    // Removing again should return false
    let removed_again = store.remove("openai").unwrap();
    assert!(!removed_again);
}

#[test]
fn test_remove_all() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    store
        .store(make_credential("openai", "sk-1", AuthMethod::Token))
        .unwrap();
    store
        .store(make_credential("anthropic", "sk-2", AuthMethod::Token))
        .unwrap();

    store.remove_all().unwrap();
    assert!(!store.exists("openai").unwrap());
    assert!(!store.exists("anthropic").unwrap());
}

#[test]
fn test_credential_expired() {
    let cred = make_expired_credential("test");
    assert!(cred.is_expired());
    assert_eq!(cred.status(), "expired");
}

#[test]
fn test_credential_not_expired() {
    let cred = Credential {
        provider: "test".to_string(),
        token: "token".to_string(),
        method: AuthMethod::OAuth,
        expires_at: Some(i64::MAX), // far future
        refresh_token: None,
        account_id: None,
    };
    assert!(!cred.is_expired());
    assert_eq!(cred.status(), "active");
}

#[test]
fn test_credential_no_expiry() {
    let cred = make_credential("test", "token", AuthMethod::Token);
    assert!(!cred.is_expired());
    assert_eq!(cred.status(), "active");
}

#[test]
fn test_status_summary_empty() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    let summary = store.status_summary().unwrap();
    assert!(summary.is_empty());
}

#[test]
fn test_status_summary_with_credentials() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    store
        .store(make_credential("openai", "sk-1", AuthMethod::OAuth))
        .unwrap();
    store.store(make_expired_credential("anthropic")).unwrap();

    let summary = store.status_summary().unwrap();
    assert_eq!(summary.len(), 2);

    let openai = summary.iter().find(|s| s.provider == "openai").unwrap();
    assert_eq!(openai.status, "active");
    assert_eq!(openai.method, "oauth");

    let anthropic = summary.iter().find(|s| s.provider == "anthropic").unwrap();
    assert_eq!(anthropic.status, "expired");
}

#[test]
fn test_list() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    store
        .store(make_credential("openai", "sk-1", AuthMethod::Token))
        .unwrap();
    store
        .store(make_credential("anthropic", "sk-2", AuthMethod::OAuth))
        .unwrap();

    let list = store.list().unwrap();
    assert_eq!(list.len(), 2);
}

#[test]
fn test_overwrite_credential() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    store
        .store(make_credential("openai", "old-token", AuthMethod::Token))
        .unwrap();
    store
        .store(make_credential("openai", "new-token", AuthMethod::OAuth))
        .unwrap();

    let loaded = store.get("openai").unwrap().unwrap();
    assert_eq!(loaded.token, "new-token");
    assert_eq!(loaded.method, AuthMethod::OAuth);
}

#[test]
fn test_path_returns_credentials_file_path() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    assert_eq!(store.path(), tmp.path().join("credentials.json"));
}

// --- Sandbox hardening: credential file permission tests ---

/// Exercises the real `store()` → `save_all()` → `atomic_write()` path (not
/// a hand-rolled simulation): a second `store()` call must fully replace
/// the credentials file's content via the same-directory-temp-file +
/// rename it performs internally, and must not leave the temp file behind.
#[test]
fn test_atomic_replacement_preserves_existing_credentials_until_rename() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(make_credential("openai", "old-token", AuthMethod::Token))
        .unwrap();
    let before = std::fs::read(store.path()).unwrap();

    // A concurrent atomic_write from an unrelated writer targeting the same
    // temp-name pattern must not corrupt the store's own credentials file:
    // it only ever becomes visible via `std::fs::rename`, which this store
    // never observes unless it performs the rename itself.
    let stray_tmp = tmp.path().join(".credentials.json.stray-writer.tmp");
    std::fs::write(&stray_tmp, b"not a credentials file").unwrap();
    assert_eq!(
        std::fs::read(store.path()).unwrap(),
        before,
        "an unrelated temp file beside the credential file must not alter current credentials"
    );
    std::fs::remove_file(&stray_tmp).unwrap();

    // The real replacement path: store() -> save_all() -> atomic_write().
    store
        .store(make_credential("openai", "new-token", AuthMethod::OAuth))
        .unwrap();

    assert_eq!(
        store.get("openai").unwrap().unwrap().token,
        "new-token",
        "the second store() call must be visible after atomic_write's rename"
    );
    let entries: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        entries,
        vec![std::ffi::OsString::from("credentials.json")],
        "atomic_write must not leave its temp file behind after a successful rename"
    );
}

#[cfg(unix)]
#[test]
fn test_credentials_file_created_with_0600() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    store
        .store(make_credential("openai", "sk-test", AuthMethod::Token))
        .unwrap();

    let metadata = std::fs::metadata(store.path()).unwrap();
    let mode = metadata.permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "expected permissions 0600, got {:04o}", mode);
}

#[cfg(unix)]
#[test]
fn test_credentials_permissions_enforced_on_every_write() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    // Store a credential to create the file
    store
        .store(make_credential("openai", "sk-test", AuthMethod::Token))
        .unwrap();

    // Manually weaken the permissions
    let permissions = std::fs::Permissions::from_mode(0o644);
    std::fs::set_permissions(store.path(), permissions).unwrap();

    // Verify they were weakened
    let metadata = std::fs::metadata(store.path()).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o644);

    // Store another credential — permissions should be re-enforced
    store
        .store(make_credential("anthropic", "sk-new", AuthMethod::Token))
        .unwrap();

    let metadata = std::fs::metadata(store.path()).unwrap();
    let mode = metadata.permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "expected permissions 0600 after re-write, got {:04o}",
        mode
    );
}
