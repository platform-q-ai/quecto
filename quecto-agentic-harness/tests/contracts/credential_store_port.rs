//! Contract for the auth capability's `CredentialStorePort` (#1839, #1842)
//! over `credentials.json`: a store makes the credential its provider's one
//! credential, leaves every other provider's untouched, keeps the file's
//! format and owner-only mode, and refuses — changing nothing — when the
//! file cannot be read; a listing returns every stored credential whole,
//! writes nothing, and refuses an unreadable file; a removal takes its
//! provider's credential alone, says whether there was one, and refuses —
//! changing nothing — when the file cannot be read.
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
    store.store(token("anthropic", "fake-a")).unwrap();
    assert!(store.remove("openai").unwrap());
    let left = store.list().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].provider, "anthropic");
    assert!(!store.remove("openai").unwrap(), "nothing left to remove");
}

#[test]
fn removing_from_an_empty_store_removes_nothing() {
    let base = TempDir::new().unwrap();
    assert!(!under_test(&base).remove("openai").unwrap());
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
