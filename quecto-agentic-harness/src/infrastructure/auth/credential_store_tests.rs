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
    let mut entries: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        vec![
            std::ffi::OsString::from("credentials.json"),
            // The cross-process lock sidecar (#1460) legitimately remains.
            std::ffi::OsString::from("credentials.json.lock"),
        ],
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

// ─── #1460: cross-process file lock around load-mutate-store ────────────────

/// A store() creates the lock file beside credentials.json and releases the
/// lock when done: after store() returns, an exclusive non-blocking lock on
/// the same file must succeed. Paired with the blocking test below, this
/// pins acquire-and-release.
#[test]
fn test_store_creates_and_releases_lock_file() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    store
        .store(make_credential("openai", "sk-test", AuthMethod::Token))
        .unwrap();

    assert!(
        store.lock_path().exists(),
        "store() must guard its load-mutate-store cycle via the lock file {}",
        store.lock_path().display()
    );
    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .open(store.lock_path())
        .unwrap();
    lock_file
        .try_lock()
        .expect("store() must release the credentials lock when its write completes");
}

/// While another process (simulated: another file description in this
/// process) holds an exclusive lock on the credentials lock file, a store()
/// must block instead of racing the read-modify-write.
#[test]
fn test_store_blocks_while_credentials_lock_is_held() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    // Seed so a real file exists to mutate.
    store
        .store(make_credential("seed", "sk-seed", AuthMethod::Token))
        .unwrap();

    // Simulated other process: hold an exclusive lock on the lock file.
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(store.lock_path())
        .unwrap();
    lock_file.lock().unwrap();

    let done = Arc::new(AtomicBool::new(false));
    let done_writer = Arc::clone(&done);
    let dir = tmp.path().to_path_buf();
    let writer = std::thread::spawn(move || {
        let store = CredentialStore::new(&dir);
        store
            .store(make_credential("alpha", "sk-alpha", AuthMethod::Token))
            .unwrap();
        done_writer.store(true, Ordering::SeqCst);
    });

    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(
        !done.load(Ordering::SeqCst),
        "store() must wait for the credentials lock held by another locker"
    );

    lock_file.unlock().unwrap();
    drop(lock_file);
    writer.join().unwrap();
    assert!(done.load(Ordering::SeqCst));
    assert_eq!(
        store.get("alpha").unwrap().expect("alpha stored").token,
        "sk-alpha"
    );
    assert_eq!(
        store.get("seed").unwrap().expect("seed survives").token,
        "sk-seed",
        "a blocked writer must not clobber existing credentials"
    );
}

/// N concurrent load-mutate-store cycles over one file must lose no tokens.
#[test]
fn test_concurrent_stores_lose_no_tokens() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();

    let threads: Vec<_> = (0..8)
        .map(|i| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let store = CredentialStore::new(&dir);
                for round in 0..25 {
                    store
                        .store(make_credential(
                            &format!("provider-{i}"),
                            &format!("sk-{i}-{round}"),
                            AuthMethod::Token,
                        ))
                        .unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }

    let store = CredentialStore::new(&dir);
    let all = store.load_snapshot().unwrap();
    for i in 0..8 {
        assert!(
            all.contains_key(&format!("provider-{i}")),
            "provider-{i} token lost by a concurrent read-modify-write; kept: {:?}",
            all.keys().collect::<Vec<_>>()
        );
    }
}

// ─── #1460 review: rotation-aware refresh persistence ───────────────────────

fn oauth_credential(provider: &str, token: &str, refresh: &str, expires_at: i64) -> Credential {
    Credential {
        provider: provider.to_string(),
        token: token.to_string(),
        method: AuthMethod::OAuth,
        expires_at: Some(expires_at),
        refresh_token: Some(refresh.to_string()),
        account_id: None,
    }
}

fn far_future() -> i64 {
    crate::infrastructure::time::unix_timestamp_secs() + 3_600
}

/// The refresh path's normal case: the on-disk refresh token is still the one
/// this refresh consumed, so the rotated credential is persisted.
#[test]
fn test_store_refreshed_persists_when_no_concurrent_rotation() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(oauth_credential("anthropic", "at-old", "rt-1", 0))
        .unwrap();

    let refreshed = oauth_credential("anthropic", "at-new", "rt-2", far_future());
    let authoritative = store.store_refreshed(refreshed, "rt-1").unwrap();

    assert_eq!(authoritative.token, "at-new");
    let on_disk = store.get("anthropic").unwrap().unwrap();
    assert_eq!(on_disk.token, "at-new");
    assert_eq!(on_disk.refresh_token.as_deref(), Some("rt-2"));
}

/// Lost-update guard: another process already rotated the token family (the
/// on-disk refresh token no longer matches the one this refresh consumed and
/// the on-disk credential is valid). The concurrent winner's credential must
/// be kept — overwriting it would persist a competing/stale token family.
#[test]
fn test_store_refreshed_keeps_concurrently_rotated_credential() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    // Another agent's refresh landed first: rt-1 -> rt-other.
    store
        .store(oauth_credential(
            "anthropic",
            "at-other",
            "rt-other",
            far_future(),
        ))
        .unwrap();

    // This agent also refreshed from rt-1, but lost the race.
    let loser = oauth_credential("anthropic", "at-loser", "rt-loser", far_future());
    let authoritative = store.store_refreshed(loser, "rt-1").unwrap();

    assert_eq!(
        authoritative.token, "at-other",
        "the concurrent winner's credential is authoritative"
    );
    let on_disk = store.get("anthropic").unwrap().unwrap();
    assert_eq!(on_disk.token, "at-other");
    assert_eq!(on_disk.refresh_token.as_deref(), Some("rt-other"));
}

/// An expired on-disk credential never blocks a refresh persist, even when
/// its refresh token differs (e.g. corrupt or ancient state).
#[test]
fn test_store_refreshed_overwrites_expired_mismatched_credential() {
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(oauth_credential("anthropic", "at-stale", "rt-stale", 0))
        .unwrap();

    let refreshed = oauth_credential("anthropic", "at-new", "rt-new", far_future());
    let authoritative = store.store_refreshed(refreshed, "rt-1").unwrap();

    assert_eq!(authoritative.token, "at-new");
    assert_eq!(store.get("anthropic").unwrap().unwrap().token, "at-new");
}

/// The credentials lock file itself must be owner-only: a world-readable
/// lock file would let any co-resident user take the exclusive lock and
/// wedge every credential write.
#[test]
fn test_lock_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(make_credential("openai", "sk-test", AuthMethod::Token))
        .unwrap();
    let mode = std::fs::metadata(store.lock_path())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "lock file must be 0600, got {mode:04o}");
}
