//! Contract for the configuration capability's `StoredCredentialPresence`
//! (#1843), answered by the auth capability's status (#1842): every stored
//! credential by provider, method and state, ordered by provider, with no
//! key material; a store that cannot be read yields only the kind of
//! failure, never its words (#2617).
use quecto::application::configuration::ports::{
    StoredCredential, StoredCredentialPresence, StoredCredentialsUnavailable,
};
use quecto::composition::bootstrap::auth::build_auth_status;
use quecto::composition::bootstrap::credential_presence::AuthStatusPresence;
use quecto::domain::auth::credential::{AuthMethod, Credential};
use quecto::infrastructure::auth::credential_store::CredentialStore;

fn presence(base: &std::path::Path) -> AuthStatusPresence {
    AuthStatusPresence::new(build_auth_status(base))
}

fn credential(provider: &str, method: AuthMethod, expires_at: Option<i64>) -> Credential {
    Credential {
        provider: provider.to_string(),
        token: format!("fake-{provider}-token"),
        method,
        expires_at,
        refresh_token: None,
        account_id: None,
    }
}

#[test]
fn an_empty_store_holds_nothing() {
    let base = tempfile::TempDir::new().unwrap();
    assert_eq!(presence(base.path()).stored(), Ok(Vec::new()));
}

#[test]
fn each_stored_credential_reports_its_method_and_state_in_provider_order() {
    let base = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(base.path());
    store
        .store(credential("xai", AuthMethod::OAuth, Some(0)))
        .unwrap();
    store
        .store(credential("anthropic", AuthMethod::Token, None))
        .unwrap();
    let stored = presence(base.path()).stored().unwrap();
    assert_eq!(
        stored,
        [
            StoredCredential {
                provider: "anthropic".to_string(),
                method: "token".to_string(),
                state: "active".to_string(),
            },
            StoredCredential {
                provider: "xai".to_string(),
                method: "oauth".to_string(),
                state: "expired".to_string(),
            },
        ]
    );
    assert!(!format!("{stored:?}").contains("fake-"), "no key material");
}

#[test]
fn a_malformed_store_yields_its_kind_and_never_its_content() {
    let base = tempfile::TempDir::new().unwrap();
    std::fs::write(
        base.path().join("credentials.json"),
        r#"{"credentials":{"openai":{"provider":"openai","token":12345678,"method":"sk-FAKE-METHOD","expires_at":null}}}"#,
    )
    .unwrap();
    assert_eq!(
        presence(base.path()).stored(),
        Err(StoredCredentialsUnavailable::Malformed)
    );
}

#[cfg(unix)]
#[test]
fn a_store_that_may_not_be_read_yields_permission_denied() {
    use std::os::unix::fs::PermissionsExt;
    let base = tempfile::TempDir::new().unwrap();
    let path = base.path().join("credentials.json");
    std::fs::write(&path, r#"{"credentials":{}}"#).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Root reads anything: the case cannot arise there.
    if std::fs::read(&path).is_ok() {
        return;
    }
    assert_eq!(
        presence(base.path()).stored(),
        Err(StoredCredentialsUnavailable::PermissionDenied)
    );
}

#[test]
fn a_store_that_cannot_be_read_otherwise_is_unreadable() {
    let base = tempfile::TempDir::new().unwrap();
    std::fs::create_dir(base.path().join("credentials.json")).unwrap();
    assert_eq!(
        presence(base.path()).stored(),
        Err(StoredCredentialsUnavailable::Unreadable)
    );
}
