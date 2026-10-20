use super::*;
use crate::domain::auth::credential::{AuthMethod, Credential};
use crate::infrastructure::auth::credential_store::CredentialStore;
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

/// A context whose `auth login` reaches the composed use cases.
fn composed_ctx() -> CliContext {
    CliContext {
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
        ..Default::default()
    }
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
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
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
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
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
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
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
    assert_eq!(out.stdout, "Credential removed for openai\n");
    assert!(!store.exists("openai").unwrap());
}

#[test]
fn test_auth_logout_nonexistent_is_noop() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
        ..Default::default()
    };
    let out = run_with_output(args("auth logout --provider openai"), &ctx);
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "no credential found for openai\n");
    let uncomposed = run_with_output(
        args("auth logout --provider openai"),
        &CliContext { auth: None, ..ctx },
    );
    assert_eq!(uncomposed.exit_code, 1);
    assert_eq!(
        uncomposed.stderr,
        "auth logout: auth capability not composed\n"
    );
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
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
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
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
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
        stdin_data: Some("4\n".to_string()), // invalid choice
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
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
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
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
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
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
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
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
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
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
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
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
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
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
        auth: Some(crate::composition::bootstrap::auth::build_auth_handles),
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
fn a_refusal_lists_the_login_providers_in_order() {
    assert_eq!(known_providers(), "openai, anthropic, xai");
}

#[test]
fn auth_login_refuses_without_composed_auth_handles() {
    let out = run_with_output(
        args("auth login --provider openai --token sk-1"),
        &CliContext::default(),
    );
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stderr, "auth login: auth capability not composed\n");
    let device = run_with_output(
        args("auth login --provider openai --device-code"),
        &CliContext::default(),
    );
    assert_eq!(device.exit_code, 1);
    assert_eq!(device.stderr, "auth login: auth capability not composed\n");
}

#[test]
fn the_unknown_provider_refusal_keeps_its_wording() {
    let out = run_with_output(
        args("auth login --provider bogus --token sk-1"),
        &composed_ctx(),
    );
    assert_eq!(out.exit_code, 1);
    assert_eq!(
        out.stderr,
        "auth login: unknown provider 'bogus'. Known: openai, anthropic, xai\n"
    );
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
fn a_device_code_login_for_an_unknown_provider_names_no_alternative() {
    let out = run_with_output(
        args("auth login --provider weird --device-code"),
        &composed_ctx(),
    );
    assert_eq!(out.exit_code, 1);
    assert_eq!(
        out.stderr,
        "auth login: device code flow is not supported for 'weird'\n"
    );
}

#[test]
fn a_device_code_login_for_xai_is_refused_even_against_a_test_server() {
    let out = run_with_output(
        args("auth login --provider xai --device-code"),
        &CliContext {
            oauth_base_url: Some("http://127.0.0.1:1".to_string()),
            ..composed_ctx()
        },
    );
    assert_eq!(out.exit_code, 1);
    assert_eq!(
        out.stderr,
        "auth login: device code flow is not supported for 'xai' (use --oauth instead)\n"
    );
}
