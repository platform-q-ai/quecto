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
