//! Additional region-coverage tests for `auth.rs`.
//!
//! Covers the pure helpers (`extract_code_from_input`, `capitalize`) and the
//! OAuth/device-code login command flows driven through `run_with_output`
//! against a leaked-on-a-dedicated-runtime `wiremock` server (avoids the
//! nested-runtime panic that would occur if the mock shared the CLI runtime).

use super::*;
use crate::infrastructure::auth::oauth::OAuthTokenResponse;
use crate::interface::cli::run_with_output;

fn args(s: &str) -> Vec<String> {
    let mut v = vec!["quecto".to_string()];
    v.extend(s.split_whitespace().map(String::from));
    v
}

/// Start a wiremock server that answers `POST <path>` with `status`/`body`,
/// then leak the server + its runtime so it outlives the CLI's own runtime.
fn leak_mock(path: &str, status: u16, body: serde_json::Value) -> String {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let path = path.to_string();
    let uri = rt.block_on(async move {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path(path.as_str()))
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

/// Build a fake JWT access token carrying a chatgpt account id.
fn jwt_with_account_id(acct: &str) -> String {
    use base64::Engine;
    let payload = serde_json::json!({
        "https://api.openai.com/auth": { "chatgpt_account_id": acct }
    });
    let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&payload).unwrap());
    format!("header.{}.sig", enc)
}

fn oauth_ctx(base: &std::path::Path, uri: String, stdin: &str) -> CliContext {
    CliContext {
        base_dir: Some(base.to_path_buf()),
        oauth_base_url: Some(uri),
        stdin_data: Some(stdin.to_string()),
        ..Default::default()
    }
}

// --- extract_code_from_input / capitalize (pure) ---

#[test]
fn test_extract_code_full_url() {
    let url = "http://localhost:1455/auth/callback?code=ABC123&state=xyz";
    assert_eq!(extract_code_from_input(url), Some("ABC123".to_string()));
}

#[test]
fn test_extract_code_last_segment() {
    assert_eq!(
        extract_code_from_input("a=1&code=LAST"),
        Some("LAST".to_string())
    );
}

#[test]
fn test_extract_code_empty_code_falls_back_to_raw() {
    // `code=` is present but empty → fall through to returning raw input.
    assert_eq!(extract_code_from_input("code="), Some("code=".to_string()));
}

#[test]
fn test_extract_code_raw_value() {
    assert_eq!(
        extract_code_from_input("just-a-code"),
        Some("just-a-code".to_string())
    );
}

#[test]
fn test_extract_code_empty_is_none() {
    assert_eq!(extract_code_from_input(""), None);
}

#[test]
fn test_capitalize() {
    assert_eq!(capitalize("openai"), "Openai");
    assert_eq!(capitalize(""), "");
    assert_eq!(capitalize("a"), "A");
}

// --- OpenAI OAuth login flow ---

#[test]
fn test_openai_oauth_login_plain_token_warns() {
    let tmp = tempfile::TempDir::new().unwrap();
    let uri = leak_mock(
        "/oauth/token",
        200,
        serde_json::json!({
            "access_token": "plain-access-token",
            "refresh_token": "rt-x",
            "expires_in": 3600
        }),
    );
    let ctx = oauth_ctx(tmp.path(), uri, "the-code\n");
    let out = run_with_output(args("auth login --provider openai --oauth"), &ctx);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("OAuth credential stored successfully"));
    assert!(out.stderr.contains("warning"));

    let store = CredentialStore::new(tmp.path());
    let cred = store.get("openai").unwrap().unwrap();
    assert_eq!(cred.token, "plain-access-token");
    assert!(cred.account_id.is_none());
}

#[test]
fn test_openai_oauth_login_jwt_stores_account_id() {
    let tmp = tempfile::TempDir::new().unwrap();
    let token = jwt_with_account_id("acct-123");
    let uri = leak_mock(
        "/oauth/token",
        200,
        serde_json::json!({
            "access_token": token,
            "refresh_token": "rt-x",
            "expires_in": 3600
        }),
    );
    let ctx = oauth_ctx(tmp.path(), uri, "the-code\n");
    let out = run_with_output(args("auth login --provider openai --oauth"), &ctx);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(!out.stderr.contains("warning"));

    let store = CredentialStore::new(tmp.path());
    let cred = store.get("openai").unwrap().unwrap();
    assert_eq!(cred.account_id, Some("acct-123".to_string()));
}

#[test]
fn test_openai_oauth_login_exchange_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let uri = leak_mock("/oauth/token", 400, serde_json::json!("invalid_grant"));
    let ctx = oauth_ctx(tmp.path(), uri, "the-code\n");
    let out = run_with_output(args("auth login --provider openai --oauth"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("token exchange failed"));
}

// --- Anthropic OAuth login flow ---

#[test]
fn test_anthropic_oauth_login_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let uri = leak_mock(
        "/oauth/token",
        200,
        serde_json::json!({
            "access_token": "sk-ant-oat01-new",
            "refresh_token": "sk-ant-ort01-new",
            "expires_in": 28800
        }),
    );
    let ctx = oauth_ctx(tmp.path(), uri, "auth-code#state\n");
    let out = run_with_output(args("auth login --provider anthropic --oauth"), &ctx);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(
        out.stdout
            .contains("Anthropic OAuth credential stored successfully")
    );

    let store = CredentialStore::new(tmp.path());
    let cred = store.get("anthropic").unwrap().unwrap();
    assert_eq!(cred.token, "sk-ant-oat01-new");
}

#[test]
fn test_anthropic_oauth_login_exchange_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let uri = leak_mock("/oauth/token", 400, serde_json::json!("invalid_grant"));
    let ctx = oauth_ctx(tmp.path(), uri, "auth-code\n");
    let out = run_with_output(args("auth login --provider anthropic --oauth"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("token exchange failed"));
}

// --- Device-code login flow ---

#[test]
fn test_device_code_login_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let uri = leak_mock(
        "/device/code",
        200,
        serde_json::json!({
            "device_code": "DEV-123",
            "user_code": "ABCD-1234",
            "verification_uri": "https://example.com/device"
        }),
    );
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        oauth_base_url: Some(uri),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider openai --device-code"), &ctx);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("Enter code: ABCD-1234"));
    assert!(out.stdout.contains("example.com/device"));
}

#[test]
fn test_device_code_login_request_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let uri = leak_mock("/device/code", 500, serde_json::json!("err"));
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        oauth_base_url: Some(uri),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider openai --device-code"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("device code request failed"));
}

// --- store_oauth_credential (direct) ---

#[test]
fn test_store_oauth_credential_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let token_resp = OAuthTokenResponse {
        access_token: "tok-abc".to_string(),
        refresh_token: Some("rt-abc".to_string()),
        expires_in: 3600,
    };
    let params = OAuthStoreParams {
        provider: "openai".to_string(),
        account_id: Some("acct-1".to_string()),
        expires_at: 999,
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut out = Output {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = store_oauth_credential(&ctx, params, &token_resp, &mut out);
    assert_eq!(code, 0);
    assert!(stdout.contains("stored successfully"));

    let store = CredentialStore::new(tmp.path());
    let cred = store.get("openai").unwrap().unwrap();
    assert_eq!(cred.token, "tok-abc");
    assert_eq!(cred.account_id, Some("acct-1".to_string()));
}

// --- login arg parsing / positional handling ---

#[test]
fn test_login_positional_arg_is_ignored() {
    // A bare positional token (not a flag) hits the `_ => i += 1` arm and is
    // skipped; the remaining flags still drive a token login.
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(
        args("auth login positional --provider openai --token sk-1"),
        &ctx,
    );
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("Credential stored for openai"));
}

#[test]
fn test_login_device_code_without_provider() {
    let ctx = CliContext::default();
    let out = run_with_output(args("auth login --device-code"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr
            .contains("--provider is required when using --device-code")
    );
}

#[test]
fn test_login_oauth_unknown_provider_rejected() {
    // OAuth (non-token, non-device) path: an unknown but explicit provider is
    // rejected by resolve_provider_interactive before any stdin read.
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider bogus --oauth"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("unknown provider 'bogus'"));
}

// --- failure arms backed by a corrupt credentials.json (forces store errors) ---

/// Write an unparseable `credentials.json` so any store op fails on load.
fn corrupt_store_ctx(tmp: &tempfile::TempDir) -> CliContext {
    std::fs::write(tmp.path().join("credentials.json"), "{ not json ").unwrap();
    CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    }
}

#[test]
fn test_token_login_store_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = corrupt_store_ctx(&tmp);
    let out = run_with_output(args("auth login --provider openai --token sk-1"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("failed to store credential"));
}

#[test]
fn test_store_oauth_credential_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = corrupt_store_ctx(&tmp);
    let token_resp = OAuthTokenResponse {
        access_token: "tok".to_string(),
        refresh_token: None,
        expires_in: 3600,
    };
    let params = OAuthStoreParams {
        provider: "openai".to_string(),
        account_id: None,
        expires_at: 1,
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut out = Output {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = store_oauth_credential(&ctx, params, &token_resp, &mut out);
    assert_eq!(code, 1);
    assert!(stderr.contains("failed to store credential"));
}

#[test]
fn test_logout_remove_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = corrupt_store_ctx(&tmp);
    let out = run_with_output(args("auth logout --provider openai"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("failed to remove credential"));
}

#[test]
fn test_logout_positional_arg_ignored() {
    // Positional arg in logout hits the `_ => i += 1` arm; --provider still wins.
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth logout extra --provider openai"), &ctx);
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert!(out.stdout.contains("no credential found for openai"));
}

#[test]
fn test_status_read_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = corrupt_store_ctx(&tmp);
    let out = run_with_output(args("auth status"), &ctx);
    assert_eq!(out.exit_code, 1);
    // status reports the read failure on stdout.
    assert!(out.stdout.contains("failed to read credentials"));
}

// --- resolve_oauth_config / cmd_auth_login_oauth direct branches ---

#[test]
fn test_login_oauth_unsupported_provider_direct() {
    // No oauth_base_url + a provider OAuthConfig::for_provider can't resolve →
    // resolve_oauth_config returns None and the flow exits 1.
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut out = Output {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = cmd_auth_login_oauth(&ctx, "weird", &mut out);
    assert_eq!(code, 1);
    assert!(stderr.contains("OAuth is not supported for 'weird'"));
}

#[test]
fn test_login_oauth_generic_provider_branch() {
    // A non-anthropic/non-openai provider with a resolvable (base-url) config
    // takes the generic "Open this URL" branch and returns 0.
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        oauth_base_url: Some("http://127.0.0.1:1".to_string()),
        ..Default::default()
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut out = Output {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = cmd_auth_login_oauth(&ctx, "weird", &mut out);
    assert_eq!(code, 0, "stderr: {}", stderr);
    assert!(stdout.contains("Open this URL in your browser"));
    assert!(stdout.contains("Waiting for authorization"));
}

#[test]
fn test_device_code_unsupported_provider_resolve_none() {
    // An unknown provider has no OAuthConfig and no base-url override → the
    // device-code flow fails in resolve_oauth_config.
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider weird --device-code"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr
            .contains("device code flow is not supported for 'weird'")
    );
}

// --- extract_fallback_code: no extractable code ---

#[test]
fn test_openai_oauth_fallback_empty_code() {
    // Test-mode OpenAI OAuth skips the browser callback and reads the paste
    // fallback; an empty paste yields no code and the flow fails.
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = oauth_ctx(tmp.path(), "http://127.0.0.1:1".to_string(), "");
    let out = run_with_output(args("auth login --provider openai --oauth"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("could not extract authorization code"));
}

// --- flush_stdout: active flush branch (no stdin_data, non-empty buffer) ---

#[test]
fn test_flush_stdout_clears_buffer() {
    let ctx = CliContext::default(); // stdin_data: None → real flush path
    let mut stdout = String::from("prompt text");
    let mut stderr = String::new();
    let mut out = Output {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    flush_stdout(&ctx, &mut out);
    assert!(out.stdout.is_empty(), "buffer should be flushed/cleared");
}
