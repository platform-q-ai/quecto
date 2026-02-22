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
fn test_auth_login_missing_token() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        stdin_data: Some("\n".to_string()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider openai"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("must not be empty"));
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
fn test_auth_login_interactive_from_stdin() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        stdin_data: Some("sk-from-stdin-test\n".to_string()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider openai"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("Paste your API token"));
    assert!(out.stdout.contains("stored"));

    let store = CredentialStore::new(tmp.path());
    let cred = store.get("openai").unwrap().unwrap();
    assert_eq!(cred.token, "sk-from-stdin-test");
}

#[test]
fn test_auth_login_interactive_empty_stdin() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        stdin_data: Some("".to_string()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider openai"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("must not be empty"));
}

#[test]
fn test_auth_login_interactive_whitespace_stdin() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        stdin_data: Some("   \n".to_string()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider openai"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("must not be empty"));
}

// ===================================================================
// auth login --oauth tests
// ===================================================================

#[test]
fn test_auth_login_oauth_openai() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider openai --oauth"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("Open this URL"));
    assert!(out.stdout.contains("authorize"));
}

#[test]
fn test_auth_login_oauth_unsupported_provider() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Use oauth_base_url=None so it actually calls OAuthConfig::for_provider
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    // Anthropic doesn't support OAuth in for_provider
    let out = run_with_output(args("auth login --provider anthropic --oauth"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("not supported"));
    assert!(out.stderr.contains("anthropic"));
}

#[test]
fn test_auth_login_oauth_with_test_base_url() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        oauth_base_url: Some("http://localhost:9999".to_string()),
        ..Default::default()
    };
    let out = run_with_output(args("auth login --provider openai --oauth"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("http://localhost:9999/authorize"));
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
    assert!(out.stderr.contains("not supported"));
    assert!(out.stderr.contains("anthropic"));
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
