//! Additional region-coverage tests for `auth_import.rs`.
//!
//! Exercises `import_anthropic` and `import_openai` across their missing-key,
//! wrong-type, valid, expired (refresh) and failure branches. The OpenAI
//! expired+refresh path is pointed at a leaked `wiremock` server via the
//! `oauth_base_url` override; the Anthropic refresh path hardcodes the real
//! base URL (no override) so it is intentionally not driven through the wire.

use super::*;

fn new_rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

fn now_secs() -> i64 {
    crate::infrastructure::time::unix_timestamp_secs()
}

/// Start a wiremock server answering `POST /oauth/token`, leaked alongside its
/// runtime so it survives independently of the caller's runtime.
fn leak_token_mock(status: u16, body: serde_json::Value) -> String {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let uri = rt.block_on(async move {
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
    std::mem::forget(rt);
    uri
}

fn jwt_with_account_id(acct: &str) -> String {
    use base64::Engine;
    let payload = serde_json::json!({
        "https://api.openai.com/auth": { "chatgpt_account_id": acct }
    });
    let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&payload).unwrap());
    format!("header.{}.sig", enc)
}

// --- import_anthropic ---

#[test]
fn test_import_anthropic_missing_key() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    let rt = new_rt();
    let json = serde_json::json!({});
    let mut o = String::new();
    let mut e = String::new();
    let mut out = Output {
        stdout: &mut o,
        stderr: &mut e,
    };
    assert_eq!(import_anthropic(&json, &store, &rt, &mut out), Some(0));
}

#[test]
fn test_import_anthropic_wrong_type() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    let rt = new_rt();
    let json = serde_json::json!({ "anthropic": { "type": "api_key", "access": "x" } });
    let mut o = String::new();
    let mut e = String::new();
    let mut out = Output {
        stdout: &mut o,
        stderr: &mut e,
    };
    assert_eq!(import_anthropic(&json, &store, &rt, &mut out), Some(0));
}

#[test]
fn test_import_anthropic_valid_non_expired() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    let rt = new_rt();
    let json = serde_json::json!({
        "anthropic": {
            "type": "oauth",
            "access": "at-valid",
            "refresh": "rt-1",
            "expires": (now_secs() + 7200) * 1000
        }
    });
    let mut o = String::new();
    let mut e = String::new();
    let mut out = Output {
        stdout: &mut o,
        stderr: &mut e,
    };
    assert_eq!(import_anthropic(&json, &store, &rt, &mut out), Some(1));
    assert!(!o.contains("refreshing"));

    let cred = store.get("anthropic").unwrap().unwrap();
    assert_eq!(cred.token, "at-valid");
}

#[test]
fn test_import_anthropic_expired_empty_refresh_else_branch() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    let rt = new_rt();
    // Expired, but refresh token empty → else branch (no network refresh).
    let json = serde_json::json!({
        "anthropic": {
            "type": "oauth",
            "access": "at-expired",
            "refresh": "",
            "expires": (now_secs() - 100) * 1000
        }
    });
    let mut o = String::new();
    let mut e = String::new();
    let mut out = Output {
        stdout: &mut o,
        stderr: &mut e,
    };
    assert_eq!(import_anthropic(&json, &store, &rt, &mut out), Some(1));
    assert!(!o.contains("refreshing"));

    let cred = store.get("anthropic").unwrap().unwrap();
    assert_eq!(cred.token, "at-expired");
}

// --- import_openai ---

#[test]
fn test_import_openai_missing_key() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    let rt = new_rt();
    let json = serde_json::json!({});
    let params = OpenAiImportParams {
        store: &store,
        rt: &rt,
        oauth_base_url: None,
    };
    let mut o = String::new();
    let mut e = String::new();
    let mut out = Output {
        stdout: &mut o,
        stderr: &mut e,
    };
    assert_eq!(import_openai(&json, &params, &mut out), Some(0));
}

#[test]
fn test_import_openai_wrong_type() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    let rt = new_rt();
    let json = serde_json::json!({ "openai": { "type": "api_key", "access": "x" } });
    let params = OpenAiImportParams {
        store: &store,
        rt: &rt,
        oauth_base_url: None,
    };
    let mut o = String::new();
    let mut e = String::new();
    let mut out = Output {
        stdout: &mut o,
        stderr: &mut e,
    };
    assert_eq!(import_openai(&json, &params, &mut out), Some(0));
}

#[test]
fn test_import_openai_empty_access() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    let rt = new_rt();
    let json = serde_json::json!({ "openai": { "type": "oauth", "access": "" } });
    let params = OpenAiImportParams {
        store: &store,
        rt: &rt,
        oauth_base_url: None,
    };
    let mut o = String::new();
    let mut e = String::new();
    let mut out = Output {
        stdout: &mut o,
        stderr: &mut e,
    };
    assert_eq!(import_openai(&json, &params, &mut out), Some(0));
}

#[test]
fn test_import_openai_non_expired_empty_refresh_none() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    let rt = new_rt();
    let json = serde_json::json!({
        "openai": {
            "type": "oauth",
            "access": "at-valid",
            "refresh": "",
            "expires": (now_secs() + 7200) * 1000
        }
    });
    let params = OpenAiImportParams {
        store: &store,
        rt: &rt,
        oauth_base_url: None,
    };
    let mut o = String::new();
    let mut e = String::new();
    let mut out = Output {
        stdout: &mut o,
        stderr: &mut e,
    };
    assert_eq!(import_openai(&json, &params, &mut out), Some(1));

    let cred = store.get("openai").unwrap().unwrap();
    assert_eq!(cred.token, "at-valid");
    assert!(cred.refresh_token.is_none());
}

#[test]
fn test_import_openai_expired_refresh_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    let rt = new_rt();
    let token = jwt_with_account_id("acct-9");
    let uri = leak_token_mock(
        200,
        serde_json::json!({
            "access_token": token,
            "refresh_token": "rt-new",
            "expires_in": 3600
        }),
    );
    let json = serde_json::json!({
        "openai": {
            "type": "oauth",
            "access": "old-access",
            "refresh": "rt-old",
            "expires": (now_secs() - 100) * 1000
        }
    });
    let params = OpenAiImportParams {
        store: &store,
        rt: &rt,
        oauth_base_url: Some(uri.as_str()),
    };
    let mut o = String::new();
    let mut e = String::new();
    let mut out = Output {
        stdout: &mut o,
        stderr: &mut e,
    };
    assert_eq!(import_openai(&json, &params, &mut out), Some(1));
    assert!(o.contains("refreshing"));

    let cred = store.get("openai").unwrap().unwrap();
    assert_eq!(cred.token, token);
    assert_eq!(cred.account_id, Some("acct-9".to_string()));
}

#[test]
fn test_import_openai_expired_refresh_failure_none() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    let rt = new_rt();
    let uri = leak_token_mock(400, serde_json::json!("bad"));
    let json = serde_json::json!({
        "openai": {
            "type": "oauth",
            "access": "old-access",
            "refresh": "rt-old",
            "expires": (now_secs() - 100) * 1000
        }
    });
    let params = OpenAiImportParams {
        store: &store,
        rt: &rt,
        oauth_base_url: Some(uri.as_str()),
    };
    let mut o = String::new();
    let mut e = String::new();
    let mut out = Output {
        stdout: &mut o,
        stderr: &mut e,
    };
    assert_eq!(import_openai(&json, &params, &mut out), None);
    assert!(e.contains("failed to refresh OpenAI token"));
}

// --- store-failure branches ---

/// Build a `CredentialStore` whose base dir is actually a regular file, so any
/// write (create_dir_all on the "base") fails deterministically without perms.
fn failing_store(tmp: &tempfile::TempDir) -> CredentialStore {
    let file = tmp.path().join("not-a-dir");
    std::fs::write(&file, b"x").unwrap();
    CredentialStore::new(file)
}

#[test]
fn test_import_anthropic_store_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = failing_store(&tmp);
    let rt = new_rt();
    let json = serde_json::json!({
        "anthropic": {
            "type": "oauth",
            "access": "at-valid",
            "refresh": "rt-1",
            "expires": (now_secs() + 7200) * 1000
        }
    });
    let mut o = String::new();
    let mut e = String::new();
    let mut out = Output {
        stdout: &mut o,
        stderr: &mut e,
    };
    assert_eq!(import_anthropic(&json, &store, &rt, &mut out), Some(0));
    assert!(
        e.contains("failed to store Anthropic credential"),
        "stderr: {e}"
    );
}

#[test]
fn test_import_openai_store_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = failing_store(&tmp);
    let rt = new_rt();
    let json = serde_json::json!({
        "openai": {
            "type": "oauth",
            "access": "at-valid",
            "refresh": "rt-1",
            "expires": (now_secs() + 7200) * 1000
        }
    });
    let params = OpenAiImportParams {
        store: &store,
        rt: &rt,
        oauth_base_url: None,
    };
    let mut o = String::new();
    let mut e = String::new();
    let mut out = Output {
        stdout: &mut o,
        stderr: &mut e,
    };
    assert_eq!(import_openai(&json, &params, &mut out), Some(0));
    assert!(
        e.contains("failed to store OpenAI credential"),
        "stderr: {e}"
    );
}
