use super::*;
use crate::domain::auth::credential::AuthMethod;
use crate::domain::auth::provider::LoginProvider;

fn environment(base_dir: &std::path::Path) -> AuthEnvironment {
    AuthEnvironment {
        base_dir: base_dir.to_path_buf(),
        oauth_base_url: None,
    }
}

#[test]
fn the_token_login_stores_into_the_base_directorys_credentials() {
    let base = tempfile::TempDir::new().unwrap();
    let handles = build_auth_handles(&environment(base.path()));
    assert_eq!(
        handles.token_login.execute("xai", "fake-token"),
        Ok(LoginProvider::Xai)
    );
    let stored = CredentialStore::new(base.path())
        .get("xai")
        .unwrap()
        .expect("stored under the run's base directory");
    assert_eq!(stored.token, "fake-token");
    assert_eq!(stored.method, AuthMethod::Token);
}

#[test]
fn the_device_code_login_refuses_a_provider_without_the_flow_offline() {
    // xAI has no device-code endpoint: the refusal needs no network.
    let base = tempfile::TempDir::new().unwrap();
    let handles = build_auth_handles(&environment(base.path()));
    assert_eq!(
        handles.device_code_login.execute("xai"),
        Err(crate::application::auth::dto::LoginError::DeviceCodeUnsupported(LoginProvider::Xai))
    );
}
