use super::*;
use crate::interface::cli::{CliContext, run_with_output};

fn args(s: &str) -> Vec<String> {
    let mut v = vec!["quecto".to_string()];
    if !s.is_empty() {
        v.extend(s.split_whitespace().map(String::from));
    }
    v
}

fn default_ctx() -> CliContext {
    CliContext::default()
}

#[test]
fn test_auth_missing_subcommand() {
    let out = run_with_output(args("auth"), &default_ctx());
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("missing subcommand"));
}

// --- Auth CLI tests ---

#[test]
fn test_auth_login_stores_token_openai() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(
        args("auth login --provider openai --token sk-test-openai"),
        &ctx,
    );
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("stored"));

    let store = CredentialStore::new(tmp.path());
    assert!(store.exists("openai").unwrap());
    let cred = store.get("openai").unwrap().unwrap();
    assert_eq!(cred.token, "sk-test-openai");
}

#[test]
fn test_auth_login_stores_token_anthropic() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(
        args("auth login --provider anthropic --token sk-ant-test"),
        &ctx,
    );
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("stored"));
}

#[test]
fn test_auth_login_missing_provider() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --token sk-test"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("--provider"));
}

#[test]
fn test_auth_login_missing_token_flag_value() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --token"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("--token requires a value"));
}

#[test]
fn test_auth_logout_removes_credential() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    // First store a credential
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

    let out = run_with_output(args("auth logout --provider openai"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("removed"));
    assert!(!store.exists("openai").unwrap());
}

#[test]
fn test_auth_logout_nonexistent_is_noop() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth logout --provider openai"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("no credential"));
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
        ..Default::default()
    };
    let out = run_with_output(args("auth status"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("openai"));
    assert!(out.stdout.contains("active"));
}

#[test]
fn test_auth_status_empty() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth status"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("no credentials"));
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
        ..Default::default()
    };
    let out = run_with_output(args("auth status"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("anthropic"));
    assert!(out.stdout.contains("expired"));
}

#[test]
fn test_auth_no_subcommand() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("missing subcommand"));
}

#[test]
fn test_auth_unknown_subcommand() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth foobar"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("unknown subcommand"));
}

#[test]
fn test_auth_login_unknown_provider_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider groq --token sk-test"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("unknown provider"));
    assert!(out.stderr.contains("groq"));
}

#[test]
fn test_auth_login_empty_token_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    // We pass a token that's all whitespace
    let v = vec![
        "quecto".to_string(),
        "auth".to_string(),
        "login".to_string(),
        "--provider".to_string(),
        "openai".to_string(),
        "--token".to_string(),
        "   ".to_string(),
    ];
    let out = run_with_output(v, &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("must not be empty"));
}

#[test]
fn test_auth_login_unknown_flag_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider openai --tokn sk-test"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("unknown flag"));
    assert!(out.stderr.contains("--tokn"));
}

#[test]
fn test_auth_logout_unknown_flag_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth logout --provder openai"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("unknown flag"));
}

// ===================================================================
// auth login interactive (stdin_data) tests
// ===================================================================

#[test]
fn test_auth_login_bare_prompts_provider_choice() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        stdin_data: Some("3\n".to_string()), // invalid choice
        ..Default::default()
    };
    let out = run_with_output(args("auth login"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stdout.contains("Choose a provider"),
        "expected provider chooser, got: {}",
        out.stdout
    );
    assert!(out.stderr.contains("invalid choice"));
}

#[test]
fn test_auth_login_bare_choose_anthropic() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Choose "1" (Anthropic), then empty auth code → error
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        stdin_data: Some("1\n".to_string()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login"), &ctx);
    // Should show auth URL prompt (we can't complete OAuth in test)
    assert!(
        out.stdout.contains("claude.ai") || out.stderr.contains("must not be empty"),
        "expected OAuth flow start, got stdout={} stderr={}",
        out.stdout,
        out.stderr
    );
}

#[test]
fn test_auth_login_bare_choose_openai() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        stdin_data: Some("2\nbogus-code\n".to_string()), // Choose OpenAI, provide bogus code
        ..Default::default()
    };
    let out = run_with_output(args("auth login"), &ctx);
    // OpenAI OAuth flow prints the URL; token exchange will fail with bogus code
    assert!(
        out.stdout.contains("Open this URL") || out.stderr.contains("token exchange"),
        "expected OAuth URL or exchange error, got stdout={} stderr={}",
        out.stdout,
        out.stderr
    );
}

#[test]
fn test_auth_login_token_still_works() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(
        args("auth login --provider openai --token sk-direct-key"),
        &ctx,
    );
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("stored"));

    let store = CredentialStore::new(tmp.path());
    let cred = store.get("openai").unwrap().unwrap();
    assert_eq!(cred.token, "sk-direct-key");
}

// ===================================================================
// auth login --oauth tests
// ===================================================================

#[test]
fn test_auth_login_oauth_openai() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        stdin_data: Some("bogus-code\n".to_string()), // Skip browser, paste code
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider openai --oauth"), &ctx);
    // Should show the URL and attempt token exchange (which fails with bogus code)
    assert!(
        out.stdout.contains("Open this URL") || out.stderr.contains("token exchange"),
        "expected OAuth URL or exchange error, got stdout={} stderr={}",
        out.stdout,
        out.stderr
    );
}

#[test]
fn test_auth_login_oauth_anthropic_prompts_for_code() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        stdin_data: Some("\n".to_string()), // empty code triggers error
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider anthropic --oauth"), &ctx);
    // Should start the flow (show URL) but fail on empty code
    assert!(
        out.stdout.contains("claude.ai") || out.stderr.contains("must not be empty"),
        "expected OAuth URL or empty code error, got stdout={} stderr={}",
        out.stdout,
        out.stderr
    );
}

#[test]
fn test_auth_login_oauth_with_test_base_url() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        oauth_base_url: Some("http://localhost:9999".to_string()),
        stdin_data: Some("bogus-code\n".to_string()), // Skip browser, paste code
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider openai --oauth"), &ctx);
    // Should show test base URL in the authorize link
    assert!(
        out.stdout.contains("http://localhost:9999/authorize")
            || out.stderr.contains("token exchange"),
        "expected test URL or exchange error, got stdout={} stderr={}",
        out.stdout,
        out.stderr
    );
}

// ===================================================================
// auth login --device-code tests
// ===================================================================

#[test]
fn test_auth_login_device_code_unsupported_provider() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider anthropic --device-code"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("not supported"),
        "expected 'not supported' in stderr: {}",
        out.stderr
    );
}

// ===================================================================
// auth login flag edge cases
// ===================================================================

#[test]
fn test_auth_login_provider_missing_value() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("--provider requires a value"));
}

#[test]
fn test_auth_login_token_missing_value() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --token"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("--token requires a value"));
}

#[test]
fn test_auth_logout_provider_missing_value() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth logout --provider"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("--provider requires a value"));
}

#[test]
fn test_auth_logout_missing_provider() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth logout"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("--provider is required"));
}

// ===================================================================
// KNOWN_PROVIDERS constant test
// ===================================================================

#[test]
fn test_known_providers_constant() {
    assert!(KNOWN_PROVIDERS.contains(&"openai"));
    assert!(KNOWN_PROVIDERS.contains(&"anthropic"));
    assert!(!KNOWN_PROVIDERS.contains(&"groq"));
}

// ===================================================================
// read_stdin_line tests
// ===================================================================

#[test]
fn test_read_stdin_line_with_data() {
    let ctx = CliContext {
        stdin_data: Some("hello world\nsecond line\n".to_string()),
        ..Default::default()
    };
    let result = read_stdin_line(&ctx).unwrap();
    assert_eq!(result, "hello world");
}

#[test]
fn test_read_stdin_line_empty_data() {
    let ctx = CliContext {
        stdin_data: Some("".to_string()),
        ..Default::default()
    };
    let result = read_stdin_line(&ctx).unwrap();
    assert_eq!(result, "");
}

#[test]
fn test_read_stdin_line_single_line_no_newline() {
    let ctx = CliContext {
        stdin_data: Some("just-a-token".to_string()),
        ..Default::default()
    };
    let result = read_stdin_line(&ctx).unwrap();
    assert_eq!(result, "just-a-token");
}

// --- expires_at safety margin tests (issue #256) ---

#[test]
fn test_import_openai_stores_credential_with_safety_margin() {
    use crate::infrastructure::auth::credential_store::CredentialStore;

    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    let now = chrono::Utc::now().timestamp();
    // expires_s in the JSON is in milliseconds, divide by 1000 gives seconds
    let expires_ms = (now + 7200) * 1000;
    let auth_json: serde_json::Value = serde_json::json!({
        "openai": {
            "type": "oauth",
            "access": "eyJ-test-token",
            "refresh": "rt-test",
            "expires": expires_ms
        }
    });

    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut out = Output {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    super::auth_import::import_openai(&auth_json, &store, &mut out);

    let creds = store.load_snapshot().unwrap();
    let cred = creds.get("openai").unwrap();
    // The stored expires_at should have a ~300-second safety margin applied
    let expected_without_margin = now + 7200;
    let expected_with_margin = expected_without_margin - 300;
    assert!(
        (cred.expires_at.unwrap() - expected_with_margin).abs() <= 2,
        "expected expires_at ~{} (with 300s margin), got {} (diff: {}s)",
        expected_with_margin,
        cred.expires_at.unwrap(),
        (cred.expires_at.unwrap() - expected_with_margin).abs()
    );
}
