//! `auth login --import-external` end to end (#1840): the composed import
//! over an `auth.json` in a temporary directory and a local OAuth server
//! standing in for every provider. Nothing real is contacted.
use std::cell::RefCell;
use std::path::PathBuf;

use crate::domain::auth::credential::AuthMethod;
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::interface::cli::auth::{AuthEnvironment, AuthHandles};
use crate::interface::cli::{CliContext, CliOutput, run_with_output};

thread_local! {
    /// The `auth.json` the rig's import reads, for the command running on
    /// this thread.
    static AUTH_JSON: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

fn rig(environment: &AuthEnvironment) -> AuthHandles {
    let auth_json = AUTH_JSON
        .with(|path| path.borrow().clone())
        .expect("the test names its auth.json");
    crate::composition::bootstrap::auth::build_auth_handles_importing_from(environment, &auth_json)
}

/// A wiremock token endpoint answering every `POST /oauth/token` with
/// `status`/`body`, leaked with its runtime so it outlives the command's.
fn token_server(status: u16, body: serde_json::Value) -> String {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let uri = runtime.block_on(async move {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/oauth/token"))
            .respond_with(wiremock::ResponseTemplate::new(status).set_body_json(&body))
            .mount(&server)
            .await;
        let uri = server.uri();
        let _: &'static wiremock::MockServer = Box::leak(Box::new(server));
        uri
    });
    std::mem::forget(runtime);
    uri
}

fn now() -> i64 {
    crate::infrastructure::time::unix_timestamp_secs()
}

/// Run the import with `auth_json` (written, unless `None`) beside a fresh
/// base directory.
fn import(
    auth_json: Option<&str>,
    oauth_base_url: Option<String>,
) -> (tempfile::TempDir, PathBuf, CliOutput) {
    let base = tempfile::TempDir::new().unwrap();
    let path = base.path().join("external").join("auth.json");
    if let Some(content) = auth_json {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
    }
    let ctx = CliContext {
        base_dir: Some(base.path().to_path_buf()),
        oauth_base_url,
        auth: Some(rig),
        ..Default::default()
    };
    AUTH_JSON.with(|slot| *slot.borrow_mut() = Some(path.clone()));
    let out = run_with_output(
        ["quecto", "auth", "login", "--import-external"]
            .map(String::from)
            .to_vec(),
        &ctx,
    );
    AUTH_JSON.with(|slot| *slot.borrow_mut() = None);
    (base, path, out)
}

#[test]
fn both_providers_import_and_an_expired_one_is_refreshed_first() {
    let uri = token_server(
        200,
        serde_json::json!({ "access_token": "fake-refreshed", "expires_in": 3600 }),
    );
    let auth_json = serde_json::json!({
        "anthropic": { "type": "oauth", "access": "fake-a", "refresh": "fake-ra",
                       "expires": (now() + 7_200) * 1000 },
        "openai": { "type": "oauth", "access": "fake-o", "refresh": "fake-ro",
                    "expires": (now() - 100) * 1000 },
        "xai": { "type": "oauth", "access": "fake-x", "refresh": "", "expires": 0 },
    })
    .to_string();
    let (base, _, out) = import(Some(&auth_json), Some(uri));
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout,
        "Imported Anthropic OAuth credential\n\
         OpenAI token expired, refreshing...\n\
         Imported OpenAI OAuth credential\n\
         Imported 2 credential(s)\n"
    );
    assert_eq!(out.stderr, "");
    let stored = CredentialStore::new(base.path()).load_snapshot().unwrap();
    assert_eq!(stored.len(), 2, "xAI is not imported from auth.json");
    assert_eq!(stored["anthropic"].token, "fake-a");
    assert_eq!(stored["anthropic"].method, AuthMethod::OAuth);
    assert_eq!(stored["openai"].token, "fake-refreshed");
    assert_eq!(
        stored["openai"].refresh_token.as_deref(),
        Some("fake-ro"),
        "a refresh without a new refresh token keeps the old one"
    );
}

#[test]
fn a_missing_auth_json_is_named() {
    let (_base, path, out) = import(None, None);
    assert_eq!(out.exit_code, 1);
    assert_eq!(
        out.stderr,
        format!("auth login: auth.json not found at {}\n", path.display())
    );
}

#[test]
fn a_malformed_auth_json_is_refused() {
    let (_base, _, out) = import(Some("{ not json"), None);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr
            .starts_with("auth login: failed to parse auth.json: "),
        "{}",
        out.stderr
    );
}

#[test]
fn an_auth_json_without_oauth_entries_imports_nothing() {
    let auth_json = r#"{"openai": {"type": "api", "key": "fake"}}"#;
    let (_base, _, out) = import(Some(auth_json), None);
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "auth login: no OAuth credentials found in auth.json\n"
    );
}

#[test]
fn a_failed_refresh_ends_the_import_with_its_reason() {
    let uri = token_server(400, serde_json::json!("fake-leaky-internals"));
    let auth_json = serde_json::json!({
        "openai": { "type": "oauth", "access": "fake-o", "refresh": "fake-ro",
                    "expires": (now() - 100) * 1000 },
    })
    .to_string();
    let (base, _, out) = import(Some(&auth_json), Some(uri));
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stdout, "OpenAI token expired, refreshing...\n");
    assert_eq!(
        out.stderr,
        "auth login: failed to refresh OpenAI token: provider error: OpenAI token refresh failed (400)\n"
    );
    assert!(!base.path().join("credentials.json").exists());
}

#[test]
fn a_failed_store_is_reported_per_provider() {
    let auth_json = serde_json::json!({
        "anthropic": { "type": "oauth", "access": "fake-a", "refresh": "",
                       "expires": (now() + 7_200) * 1000 },
    })
    .to_string();
    let base = tempfile::TempDir::new().unwrap();
    let path = base.path().join("auth.json");
    std::fs::write(&path, auth_json).unwrap();
    std::fs::write(base.path().join("credentials.json"), "{ not json").unwrap();
    let ctx = CliContext {
        base_dir: Some(base.path().to_path_buf()),
        auth: Some(rig),
        ..Default::default()
    };
    AUTH_JSON.with(|slot| *slot.borrow_mut() = Some(path));
    let out = run_with_output(
        ["quecto", "auth", "login", "--import-external"]
            .map(String::from)
            .to_vec(),
        &ctx,
    );
    AUTH_JSON.with(|slot| *slot.borrow_mut() = None);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.starts_with(
            "auth login: failed to store Anthropic credential: config error: failed to parse credentials"
        ),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr
            .ends_with("auth login: no OAuth credentials found in auth.json\n")
    );
}
