use std::path::PathBuf;

use super::*;

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

fn assert_contains_all(haystack: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            haystack.contains(needle),
            "expected output to contain '{needle}', got:\n{haystack}"
        );
    }
}

#[test]
fn test_no_args_triggers_repl_mode() {
    // run_with_output with no args delegates to run_repl_with_output,
    // which enters REPL mode with empty input (exits immediately on EOF).
    // Without a config file, the REPL outputs an error.
    let out = run_with_output(vec!["quecto".to_string()], &default_ctx());
    // Either exits 0 (with config) or 1 (without config, showing config error).
    // In default context without config, exit code is 1.
    assert!(out.exit_code == 0 || out.exit_code == 1);
}

#[test]
fn test_help_command_shows_usage() {
    let out = run_with_output(args("help"), &default_ctx());
    assert_eq!(out.exit_code, 0);
    assert_contains_all(
        &out.stdout,
        &[
            "Usage: quecto [command]",
            "agent",
            "status",
            "auth",
            "skills",
            "help",
            "version",
        ],
    );
}

#[test]
fn test_version_command() {
    let out = run_with_output(args("version"), &default_ctx());
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("quecto"));
    assert!(out.stdout.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn test_version_flag() {
    let out = run_with_output(args("--version"), &default_ctx());
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("quecto"));
}

#[test]
fn test_version_short_flag() {
    let out = run_with_output(args("-v"), &default_ctx());
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("quecto"));
}

#[test]
fn test_unknown_command() {
    let out = run_with_output(args("foobar"), &default_ctx());
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("Unknown command: foobar"));
    assert!(out.stdout.contains("Usage: quecto [command]"));
}

#[test]
fn test_help_text_includes_all_commands() {
    let mut out = String::new();
    help_text(&mut out);
    assert_contains_all(
        &out,
        &["agent", "auth", "status", "skills", "help", "version"],
    );
}

#[test]
fn test_version_text_includes_semver() {
    let mut out = String::new();
    version_text(&mut out);
    assert!(out.starts_with("quecto "));
    // Should match semver pattern
    let version_part = out.trim().strip_prefix("quecto ").unwrap();
    let parts: Vec<&str> = version_part.split('.').collect();
    assert_eq!(parts.len(), 3, "expected semver, got: {}", version_part);
}

#[test]
fn test_cli_context_default_base_dir() {
    let ctx = CliContext::default();
    let base = ctx.base_dir();
    // Should end with .quecto (either from home dir or fallback)
    assert!(
        base.to_string_lossy().contains(".quecto") || base.to_string_lossy().contains("quecto"),
        "base dir should contain 'quecto': {}",
        base.display()
    );
}

#[test]
fn test_cli_context_override_base_dir() {
    let ctx = CliContext {
        base_dir: Some(PathBuf::from("/tmp/test-quecto")),
        ..Default::default()
    };
    assert_eq!(ctx.base_dir(), PathBuf::from("/tmp/test-quecto"));
}

#[test]
fn test_session_name_validation() {
    assert!(is_valid_session_name("my-chat"));
    assert!(is_valid_session_name("chat_1"));
    assert!(is_valid_session_name("ALLCAPS"));
    assert!(is_valid_session_name("-")); // ephemeral
    assert!(!is_valid_session_name("../../tmp/evil"));
    assert!(!is_valid_session_name("foo/bar"));
    assert!(!is_valid_session_name(".."));
    assert!(!is_valid_session_name(""));
    assert!(!is_valid_session_name("a b")); // spaces
    assert!(!is_valid_session_name("a:b")); // colons
}

// ===================================================================
// parse_repl_flags() tests
// ===================================================================

#[test]
fn test_parse_repl_flags_no_args() {
    let flags = parse_repl_flags(&[]).unwrap();
    assert!(flags.session_name.is_none());
    assert!(flags.system_prompt.is_none());
    assert!(flags.model_override.is_none());
}

#[test]
fn test_parse_repl_flags_session_short() {
    let args: Vec<String> = vec!["-s".into(), "my-session".into()];
    let flags = parse_repl_flags(&args).unwrap();
    assert_eq!(flags.session_name.as_deref(), Some("my-session"));
}

#[test]
fn test_parse_repl_flags_session_long() {
    let args: Vec<String> = vec!["--session".into(), "my-session".into()];
    let flags = parse_repl_flags(&args).unwrap();
    assert_eq!(flags.session_name.as_deref(), Some("my-session"));
}

#[test]
fn test_parse_repl_flags_session_ephemeral() {
    let args: Vec<String> = vec!["-s".into(), "-".into()];
    let flags = parse_repl_flags(&args).unwrap();
    assert_eq!(flags.session_name.as_deref(), Some("-"));
}

#[test]
fn test_parse_repl_flags_session_missing_value() {
    let args: Vec<String> = vec!["-s".into()];
    let result = parse_repl_flags(&args);
    match result {
        Err(msg) => assert!(msg.contains("-s requires a session name"), "got: {msg}"),
        Ok(_) => panic!("expected error"),
    }
}

#[test]
fn test_parse_repl_flags_session_invalid_name() {
    let args: Vec<String> = vec!["-s".into(), "../../evil".into()];
    let result = parse_repl_flags(&args);
    match result {
        Err(msg) => assert!(msg.contains("alphanumeric"), "got: {msg}"),
        Ok(_) => panic!("expected error"),
    }
}

#[test]
fn test_parse_repl_flags_system_prompt() {
    let args: Vec<String> = vec!["--system".into(), "You are helpful".into()];
    let flags = parse_repl_flags(&args).unwrap();
    assert_eq!(flags.system_prompt.as_deref(), Some("You are helpful"));
}

#[test]
fn test_parse_repl_flags_system_missing_value() {
    let args: Vec<String> = vec!["--system".into()];
    let result = parse_repl_flags(&args);
    match result {
        Err(msg) => assert!(msg.contains("--system requires a value"), "got: {msg}"),
        Ok(_) => panic!("expected error"),
    }
}

#[test]
fn test_parse_repl_flags_model_override() {
    let args: Vec<String> = vec!["--model".into(), "gpt-4o".into()];
    let flags = parse_repl_flags(&args).unwrap();
    assert_eq!(flags.model_override.as_deref(), Some("gpt-4o"));
}

#[test]
fn test_parse_repl_flags_model_missing_value() {
    let args: Vec<String> = vec!["--model".into()];
    let result = parse_repl_flags(&args);
    match result {
        Err(msg) => assert!(msg.contains("--model requires a value"), "got: {msg}"),
        Ok(_) => panic!("expected error"),
    }
}

#[test]
fn test_parse_repl_flags_unknown_flag() {
    let args: Vec<String> = vec!["--foobar".into()];
    let result = parse_repl_flags(&args);
    match result {
        Err(msg) => {
            assert!(msg.contains("unknown flag"), "got: {msg}");
            assert!(msg.contains("--foobar"), "got: {msg}");
        }
        Ok(_) => panic!("expected error"),
    }
}

#[test]
fn test_parse_repl_flags_unknown_short_flag() {
    let args: Vec<String> = vec!["-x".into()];
    let result = parse_repl_flags(&args);
    match result {
        Err(msg) => assert!(msg.contains("unknown flag"), "got: {msg}"),
        Ok(_) => panic!("expected error"),
    }
}

#[test]
fn test_parse_repl_flags_all_combined() {
    let args: Vec<String> = vec![
        "-s".into(),
        "chat1".into(),
        "--system".into(),
        "Be brief".into(),
        "--model".into(),
        "claude-3".into(),
    ];
    let flags = parse_repl_flags(&args).unwrap();
    assert_eq!(flags.session_name.as_deref(), Some("chat1"));
    assert_eq!(flags.system_prompt.as_deref(), Some("Be brief"));
    assert_eq!(flags.model_override.as_deref(), Some("claude-3"));
}

#[test]
fn test_parse_repl_flags_non_flag_args_ignored() {
    // Non-flag arguments (not starting with - or --) are skipped
    let args: Vec<String> = vec!["some_positional".into()];
    let flags = parse_repl_flags(&args).unwrap();
    assert!(flags.session_name.is_none());
}

// ===================================================================
// run_repl_with_output() tests
// ===================================================================

#[test]
fn test_repl_with_output_no_config_uses_defaults() {
    // Zero-config: no config file loads defaults; with no provider key the REPL
    // fails on the missing provider, not on a missing config / onboarding step.
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_repl_with_output(&ctx, &[], &[], false);
    assert_eq!(out.exit_code, 1);
    assert!(!out.stdout.contains("Config not found"));
    assert!(
        out.stdout.contains("no LLM providers configured")
            || out.stderr.contains("no LLM providers configured"),
        "stdout: {}, stderr: {}",
        out.stdout,
        out.stderr
    );
}

#[test]
fn test_repl_with_output_bad_flags() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let bad_args: Vec<String> = vec!["--unknown-flag".into()];
    let out = run_repl_with_output(&ctx, &bad_args, &[], false);
    assert_eq!(out.exit_code, 1);
    assert!(out.stdout.contains("Error:"));
    assert!(out.stdout.contains("unknown flag"));
}

#[test]
fn test_repl_with_output_invalid_session_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let bad_args: Vec<String> = vec!["-s".into(), "../evil".into()];
    let out = run_repl_with_output(&ctx, &bad_args, &[], false);
    assert_eq!(out.exit_code, 1);
    assert!(out.stdout.contains("Error:"));
}

#[test]
fn test_repl_with_output_no_providers() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Config with empty API keys
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":""},"anthropic":{"api_key":""}}}"#,
    )
    .unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_repl_with_output(&ctx, &[], &[], false);
    assert_eq!(out.exit_code, 1);
    assert!(out.stdout.contains("no LLM providers"));
}

// ===================================================================
// run_with_output (main dispatcher) edge cases
// ===================================================================

#[test]
fn test_run_with_output_empty_args() {
    // Just the binary name, no subcommand — triggers REPL mode
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(vec!["quecto".into()], &ctx);
    // Without a config it exits with 1
    assert_eq!(out.exit_code, 1);
}

#[test]
fn test_run_with_output_multiple_unknown_commands() {
    let out = run_with_output(args("notacommand"), &default_ctx());
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("Unknown command"));
    // Help text is also shown
    assert!(out.stdout.contains("Usage:"));
}

// ===================================================================
// CliContext tests
// ===================================================================

#[test]
fn test_cli_context_stdin_data() {
    let ctx = CliContext {
        stdin_data: Some("test data".to_string()),
        ..Default::default()
    };
    assert_eq!(ctx.stdin_data.as_deref(), Some("test data"));
}

#[test]
fn test_cli_context_oauth_base_url() {
    let ctx = CliContext {
        oauth_base_url: Some("http://test.local".to_string()),
        ..Default::default()
    };
    assert_eq!(ctx.oauth_base_url.as_deref(), Some("http://test.local"));
}

#[test]
fn test_cli_context_all_fields() {
    let ctx = CliContext {
        base_dir: Some(PathBuf::from("/tmp/test")),
        config_path: None,
        stdin_data: Some("input".to_string()),
        oauth_base_url: Some("http://oauth.local".to_string()),
        github_raw_base_url: Some("http://raw.local".to_string()),
    };
    assert_eq!(ctx.base_dir(), PathBuf::from("/tmp/test"));
    assert_eq!(ctx.stdin_data.as_deref(), Some("input"));
    assert_eq!(ctx.oauth_base_url.as_deref(), Some("http://oauth.local"));
    assert_eq!(ctx.github_raw_base_url.as_deref(), Some("http://raw.local"));
}

// ===================================================================
// CliOutput struct tests
// ===================================================================

#[test]
fn test_cli_output_debug() {
    let out = CliOutput {
        stdout: "hello".into(),
        stderr: "err".into(),
        exit_code: 0,
    };
    let debug = format!("{:?}", out);
    assert!(debug.contains("hello"));
    assert!(debug.contains("err"));
    assert!(debug.contains("0"));
}

#[test]
fn test_cli_output_clone() {
    let out = CliOutput {
        stdout: "output".into(),
        stderr: "error".into(),
        exit_code: 42,
    };
    let cloned = out.clone();
    assert_eq!(cloned.stdout, "output");
    assert_eq!(cloned.stderr, "error");
    assert_eq!(cloned.exit_code, 42);
}

// ===================================================================
// CLI flag aliases
// ===================================================================

#[test]
fn test_help_flag_alias() {
    let out = run_with_output(args("--help"), &default_ctx());
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("Usage: quecto"));
}

#[test]
fn test_help_short_flag_alias() {
    let out = run_with_output(args("-h"), &default_ctx());
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("Usage: quecto"));
}

// ===================================================================
// session name validation edge cases
// ===================================================================

#[test]
fn test_session_name_with_underscores() {
    assert!(is_valid_session_name("my_session_name"));
}

#[test]
fn test_session_name_with_numbers() {
    assert!(is_valid_session_name("session123"));
}

#[test]
fn test_session_name_all_dashes() {
    assert!(is_valid_session_name("---"));
}

#[test]
fn test_session_name_single_char() {
    assert!(is_valid_session_name("a"));
}

#[test]
fn test_session_name_with_dots_rejected() {
    assert!(!is_valid_session_name("my.session"));
}

#[test]
fn test_session_name_with_backslash_rejected() {
    assert!(!is_valid_session_name("a\\b"));
}

// ===================================================================
// build_tokio_runtime() tests
// ===================================================================

#[test]
fn test_build_tokio_runtime_succeeds() {
    let rt = build_tokio_runtime();
    assert!(rt.is_ok(), "build_tokio_runtime should succeed");
    // Verify the runtime is functional by running a trivial async task.
    let rt = rt.unwrap();
    let result = rt.block_on(async { 42 });
    assert_eq!(result, 42);
}

// ===================================================================
// --no-sandbox flag tests for REPL
// ===================================================================

#[test]
fn test_parse_repl_flags_no_sandbox_defaults_false() {
    let flags = parse_repl_flags(&[]).unwrap();
    assert!(
        !flags.no_sandbox,
        "--no-sandbox should be false by default in REPL"
    );
}

#[test]
fn test_parse_repl_flags_no_sandbox_parsed() {
    let args: Vec<String> = vec!["--no-sandbox".into()];
    let flags = parse_repl_flags(&args).unwrap();
    assert!(
        flags.no_sandbox,
        "--no-sandbox should be true when provided to REPL"
    );
}

#[test]
fn test_parse_repl_flags_no_sandbox_combined_with_session() {
    let args: Vec<String> = vec![
        "--no-sandbox".into(),
        "-s".into(),
        "chat1".into(),
        "--model".into(),
        "gpt-4".into(),
    ];
    let flags = parse_repl_flags(&args).unwrap();
    assert!(flags.no_sandbox);
    assert_eq!(flags.session_name.as_deref(), Some("chat1"));
    assert_eq!(flags.model_override.as_deref(), Some("gpt-4"));
}

// --- Issue #300: --config flag ---

#[test]
fn test_config_flag_extracted_from_args() {
    let args: Vec<String> = vec![
        "quecto".into(),
        "--config".into(),
        "/tmp/custom.json".into(),
        "agent".into(),
        "-m".into(),
        "hi".into(),
    ];
    let path = super::extract_config_flag(&args);
    assert_eq!(path, Some(std::path::PathBuf::from("/tmp/custom.json")));
}

#[test]
fn test_config_flag_not_present() {
    let args: Vec<String> = vec!["quecto".into(), "agent".into(), "-m".into(), "hi".into()];
    let path = super::extract_config_flag(&args);
    assert_eq!(path, None);
}

#[test]
fn test_cli_context_config_path_override() {
    let ctx = CliContext {
        base_dir: Some(std::path::PathBuf::from("/home/test/.quecto")),
        config_path: Some(std::path::PathBuf::from("/tmp/custom.json")),
        ..Default::default()
    };
    assert_eq!(
        ctx.config_path(),
        std::path::PathBuf::from("/tmp/custom.json")
    );
}

#[test]
fn test_cli_context_config_path_default() {
    let ctx = CliContext {
        base_dir: Some(std::path::PathBuf::from("/home/test/.quecto")),
        ..Default::default()
    };
    assert_eq!(
        ctx.config_path(),
        std::path::PathBuf::from("/home/test/.quecto/config.json")
    );
}
