//! `auth status` through the composed use case, word for word.

use crate::domain::auth::credential::{AuthMethod, Credential};
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::interface::cli::{CliContext, run_with_output};

fn args(s: &str) -> Vec<String> {
    std::iter::once("quecto")
        .chain(s.split_whitespace())
        .map(String::from)
        .collect()
}

#[test]
fn test_auth_status_shows_credentials() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "sk-test".to_string(),
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();

    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
        ..Default::default()
    };
    let out = run_with_output(args("auth status"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "Credentials:\n  openai (token) — active\n");
}

#[test]
fn test_auth_status_empty() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
        ..Default::default()
    };
    let out = run_with_output(args("auth status"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "no credentials stored\n");
}

#[test]
fn test_auth_status_expired() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "expired-tok".to_string(),
            method: AuthMethod::Token,
            expires_at: Some(0),
            refresh_token: None,
            account_id: None,
        })
        .unwrap();

    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
        ..Default::default()
    };
    let out = run_with_output(args("auth status"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "Credentials:\n  anthropic (token) — expired\n");
}

#[test]
fn auth_status_lists_providers_in_order_and_refuses_uncomposed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    for (provider, method) in [("xai", AuthMethod::OAuth), ("anthropic", AuthMethod::Token)] {
        store
            .store(Credential {
                provider: provider.to_string(),
                token: "fake".to_string(),
                method,
                expires_at: None,
                refresh_token: None,
                account_id: None,
            })
            .unwrap();
    }
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
        ..Default::default()
    };
    let out = run_with_output(args("auth status"), &ctx);
    assert_eq!(
        out.stdout,
        "Credentials:\n  anthropic (token) — active\n  xai (oauth) — active\n"
    );
    let uncomposed = run_with_output(args("auth status"), &CliContext { auth: None, ..ctx });
    assert_eq!(uncomposed.exit_code, 1);
    assert_eq!(uncomposed.stdout, "");
    assert_eq!(
        uncomposed.stderr,
        "auth status: auth capability not composed\n"
    );
}
