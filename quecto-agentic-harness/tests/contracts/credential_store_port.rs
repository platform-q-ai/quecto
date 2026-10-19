//! Contract for the auth capability's `CredentialStorePort` (#1839, #1841,
//! #1842)
//! over `credentials.json`: a store makes the credential its provider's one
//! credential, leaves every other provider's untouched, keeps the file's
//! format and owner-only mode, and refuses — changing nothing — when the
//! file cannot be read; a listing returns every stored credential whole,
//! writes nothing, and refuses an unreadable file; a removal takes its
//! provider's credential alone, says whether there was one, waits for the
//! cross-process credentials lock (#1460) so no concurrent write is lost,
//! keeps the file owner-only, and refuses — changing nothing — when the
//! file cannot be read.
use std::sync::Arc;

use tempfile::TempDir;

use quecto::application::auth::ports::CredentialStorePort;
use quecto::domain::auth::credential::{AuthMethod, Credential};
use quecto::infrastructure::auth::credential_store::CredentialStore;

fn under_test(base: &TempDir) -> Arc<dyn CredentialStorePort> {
    Arc::new(CredentialStore::new(base.path()))
}

fn oauth(provider: &str, token: &str) -> Credential {
    Credential {
        provider: provider.to_string(),
        token: token.to_string(),
        method: AuthMethod::OAuth,
        expires_at: Some(1_700_000_000),
        refresh_token: Some(format!("fake-refresh-{provider}")),
        account_id: Some("acct-fake".to_string()),
    }
}

fn token(provider: &str, token: &str) -> Credential {
    Credential {
        provider: provider.to_string(),
        token: token.to_string(),
        method: AuthMethod::Token,
        expires_at: None,
        refresh_token: None,
        account_id: None,
    }
}

fn on_disk(base: &TempDir) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(base.path().join("credentials.json")).unwrap()).unwrap()
}

#[test]
fn a_stored_credential_reads_back_whole() {
    let base = TempDir::new().unwrap();
    under_test(&base).store(oauth("openai", "fake-a")).unwrap();
    let read = CredentialStore::new(base.path())
        .get("openai")
        .unwrap()
        .expect("stored");
    assert_eq!(read.token, "fake-a");
    assert_eq!(read.method, AuthMethod::OAuth);
    assert_eq!(read.expires_at, Some(1_700_000_000));
    assert_eq!(read.refresh_token.as_deref(), Some("fake-refresh-openai"));
    assert_eq!(read.account_id.as_deref(), Some("acct-fake"));
}

#[test]
fn a_store_replaces_its_providers_credential_and_no_other() {
    let base = TempDir::new().unwrap();
    let store = under_test(&base);
    store.store(oauth("openai", "fake-old")).unwrap();
    store.store(token("anthropic", "fake-kept")).unwrap();
    store.store(token("openai", "fake-new")).unwrap();
    let all = CredentialStore::new(base.path()).load_snapshot().unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all["openai"].token, "fake-new");
    assert_eq!(all["openai"].method, AuthMethod::Token);
    assert_eq!(all["openai"].refresh_token, None);
    assert_eq!(all["anthropic"].token, "fake-kept");
}

#[test]
fn the_file_keeps_its_format() {
    let base = TempDir::new().unwrap();
    let store = under_test(&base);
    store.store(token("anthropic", "fake-t")).unwrap();
    store.store(oauth("openai", "fake-o")).unwrap();
    let file = on_disk(&base);
    assert_eq!(
        file["credentials"]["anthropic"],
        serde_json::json!({
            "provider": "anthropic",
            "token": "fake-t",
            "method": "token",
            "expires_at": null,
        })
    );
    assert_eq!(
        file["credentials"]["openai"],
        serde_json::json!({
            "provider": "openai",
            "token": "fake-o",
            "method": "oauth",
            "expires_at": 1_700_000_000,
            "refresh_token": "fake-refresh-openai",
            "account_id": "acct-fake",
        })
    );
}

/// Byte for byte, as `serde_json::to_string_pretty` wrote it before the
/// credential moved to the domain: field order included.
#[test]
fn the_file_keeps_its_exact_bytes() {
    let token_base = TempDir::new().unwrap();
    under_test(&token_base)
        .store(token("anthropic", "fake-t"))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(token_base.path().join("credentials.json")).unwrap(),
        r#"{
  "credentials": {
    "anthropic": {
      "provider": "anthropic",
      "token": "fake-t",
      "method": "token",
      "expires_at": null
    }
  }
}"#
    );
    let oauth_base = TempDir::new().unwrap();
    under_test(&oauth_base)
        .store(oauth("openai", "fake-o"))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(oauth_base.path().join("credentials.json")).unwrap(),
        r#"{
  "credentials": {
    "openai": {
      "provider": "openai",
      "token": "fake-o",
      "method": "oauth",
      "expires_at": 1700000000,
      "refresh_token": "fake-refresh-openai",
      "account_id": "acct-fake"
    }
  }
}"#
    );
}

#[cfg(unix)]
#[test]
fn the_file_is_readable_by_its_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let base = TempDir::new().unwrap();
    under_test(&base).store(token("xai", "fake-x")).unwrap();
    let mode = std::fs::metadata(base.path().join("credentials.json"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn an_unreadable_store_refuses_and_changes_nothing() {
    let base = TempDir::new().unwrap();
    let path = base.path().join("credentials.json");
    std::fs::write(&path, "{ not json ").unwrap();
    let refused = under_test(&base)
        .store(token("openai", "fake-t"))
        .expect_err("a corrupt store refuses");
    assert!(
        refused.0.contains("failed to parse credentials"),
        "{refused}"
    );
    assert!(!refused.0.contains("fake-t"), "never quotes the secret");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json ");
}

#[test]
fn a_store_with_no_file_lists_nothing() {
    let base = TempDir::new().unwrap();
    assert!(under_test(&base).list().unwrap().is_empty());
    assert!(
        !base.path().join("credentials.json").exists(),
        "listing writes nothing"
    );
}

#[test]
fn the_listing_holds_each_providers_one_credential_whole() {
    let base = TempDir::new().unwrap();
    let store = under_test(&base);
    store.store(oauth("openai", "fake-old")).unwrap();
    store.store(token("anthropic", "fake-t")).unwrap();
    store.store(oauth("openai", "fake-new")).unwrap();
    let mut listed = store.list().unwrap();
    listed.sort_by(|a, b| a.provider.cmp(&b.provider));
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].provider, "anthropic");
    assert_eq!(listed[0].method, AuthMethod::Token);
    assert_eq!(listed[1].token, "fake-new");
    assert_eq!(listed[1].expires_at, Some(1_700_000_000));
    assert_eq!(listed[1].account_id.as_deref(), Some("acct-fake"));
}

#[test]
fn an_unreadable_store_cannot_be_listed() {
    let base = TempDir::new().unwrap();
    std::fs::write(base.path().join("credentials.json"), "{ not json ").unwrap();
    let refused = under_test(&base)
        .list()
        .expect_err("a corrupt store refuses");
    assert!(
        refused.0.contains("failed to parse credentials"),
        "{refused}"
    );
}

#[test]
fn a_removal_takes_its_providers_credential_alone() {
    let base = TempDir::new().unwrap();
    let store = under_test(&base);
    store.store(oauth("openai", "fake-o")).unwrap();
    store.store(oauth("anthropic", "fake-a")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            base.path().join("credentials.json"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
    }
    assert!(store.remove("openai").unwrap());
    let left = store.list().unwrap();
    assert_eq!(left.len(), 1);
    let kept = &left[0];
    assert_eq!(kept.provider, "anthropic");
    assert_eq!(kept.token, "fake-a");
    assert_eq!(kept.method, AuthMethod::OAuth);
    assert_eq!(kept.expires_at, Some(1_700_000_000));
    assert_eq!(
        kept.refresh_token.as_deref(),
        Some("fake-refresh-anthropic")
    );
    assert_eq!(kept.account_id.as_deref(), Some("acct-fake"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(base.path().join("credentials.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "a removal rewrites the file owner-only"
        );
    }
    assert!(!store.remove("openai").unwrap(), "nothing left to remove");
}

#[test]
fn removing_from_an_empty_store_removes_nothing_and_leaves_an_empty_store() {
    let base = TempDir::new().unwrap();
    assert!(!under_test(&base).remove("openai").unwrap());
    // What the port documents a not-stored removal writes: an empty store
    // (and its lock file) where there was none.
    assert_eq!(
        std::fs::read_to_string(base.path().join("credentials.json")).unwrap(),
        "{\n  \"credentials\": {}\n}"
    );
    assert!(base.path().join("credentials.json.lock").exists());
}

#[test]
fn a_removal_waits_for_the_credentials_lock() {
    let base = TempDir::new().unwrap();
    let store = under_test(&base);
    store.store(token("openai", "fake-o")).unwrap();
    // Another process's load-mutate-store cycle holds the lock.
    let held = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(base.path().join("credentials.json.lock"))
        .unwrap();
    held.lock().unwrap();
    let (done, removed) = std::sync::mpsc::channel();
    // The remover says it has started before the window opens, so the
    // window measures waiting for the lock, not thread start-up.
    let (started, has_started) = std::sync::mpsc::channel();
    let remover = {
        let store = Arc::clone(&store);
        std::thread::spawn(move || {
            started.send(()).unwrap();
            done.send(store.remove("openai")).unwrap()
        })
    };
    has_started.recv().unwrap();
    assert!(
        removed
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_err(),
        "a removal must wait for the lock another locker holds"
    );
    // While the lock is held the file is untouched.
    assert_eq!(
        CredentialStore::new(base.path())
            .get("openai")
            .unwrap()
            .map(|credential| credential.token),
        Some("fake-o".to_string())
    );
    // The holder's own write, as a concurrent refresh would make it: the
    // removal must read the store only once it holds the lock, so this
    // write survives it.
    std::fs::write(
        base.path().join("credentials.json"),
        serde_json::json!({"credentials": {
            "openai": {"provider": "openai", "token": "fake-o", "method": "token", "expires_at": null},
            "anthropic": {"provider": "anthropic", "token": "fake-rotated", "method": "oauth", "expires_at": null, "refresh_token": "fake-refresh-rotated"},
        }})
        .to_string(),
    )
    .unwrap();
    held.unlock().unwrap();
    drop(held);
    assert_eq!(removed.recv().unwrap(), Ok(true));
    remover.join().unwrap();
    let left = store.list().unwrap();
    assert_eq!(left.len(), 1, "the holder's write survives: {left:?}");
    assert_eq!(left[0].provider, "anthropic");
    assert_eq!(left[0].token, "fake-rotated");
    assert_eq!(
        left[0].refresh_token.as_deref(),
        Some("fake-refresh-rotated")
    );
}

#[test]
fn concurrent_stores_and_a_removal_lose_nothing() {
    let base = TempDir::new().unwrap();
    let store = under_test(&base);
    store.store(token("victim", "fake-v")).unwrap();
    // All nine start together, so their cycles overlap.
    let start = Arc::new(std::sync::Barrier::new(9));
    let writers: Vec<_> = (0..8)
        .map(|n| {
            let store = Arc::clone(&store);
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                store.store(token(&format!("p{n}"), "fake-p")).unwrap()
            })
        })
        .chain(std::iter::once({
            let store = Arc::clone(&store);
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                assert!(store.remove("victim").unwrap())
            })
        }))
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    let mut providers: Vec<_> = store
        .list()
        .unwrap()
        .into_iter()
        .map(|credential| credential.provider)
        .collect();
    providers.sort();
    assert_eq!(
        providers,
        (0..8).map(|n| format!("p{n}")).collect::<Vec<_>>()
    );
}

#[test]
fn an_unreadable_store_refuses_a_removal_and_changes_nothing() {
    let base = TempDir::new().unwrap();
    let path = base.path().join("credentials.json");
    std::fs::write(&path, "{ not json ").unwrap();
    let refused = under_test(&base)
        .remove("openai")
        .expect_err("a corrupt store refuses");
    assert!(
        refused.0.contains("failed to parse credentials"),
        "{refused}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json ");
}
