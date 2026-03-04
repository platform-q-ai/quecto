use super::*;
use crate::interface::cli::{CliContext, run_with_output};

fn default_ctx() -> CliContext {
    CliContext::default()
}

// ===================================================================
// --no-sandbox flag parsing tests
// ===================================================================

#[test]
fn test_agent_no_sandbox_flag_defaults_false() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(!flags.no_sandbox, "--no-sandbox should be false by default");
}

#[test]
fn test_agent_no_sandbox_flag_parsed() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--no-sandbox".into(), "-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(
        flags.no_sandbox,
        "--no-sandbox should be true when provided"
    );
}

#[test]
fn test_agent_no_sandbox_combined_with_no_session() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--no-sandbox".into(),
        "--no-session".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.no_sandbox);
    assert!(flags.no_session);
}

#[test]
fn test_agent_no_sandbox_combined_with_session() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--no-sandbox".into(),
        "-s".into(),
        "my-session".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.no_sandbox);
    assert_eq!(flags.session_name.as_deref(), Some("my-session"));
}

#[test]
fn test_agent_help_documents_no_sandbox() {
    let out = run_with_output(vec!["quecto".into(), "help".into()], &default_ctx());
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.contains("--no-sandbox"),
        "help text must document --no-sandbox, got:\n{}",
        out.stdout
    );
}

// ===================================================================
// --no-sandbox integration: build_agent_from_config
// ===================================================================

/// Verify that --no-sandbox causes build_agent_from_config to construct an
/// unrestricted sandbox even when config has restrict_to_workspace: true.
///
/// We confirm via the stderr warning emitted when the flag is active and by
/// checking the agent builds successfully (which implies the Sandbox was
/// constructed — a sandboxed agent on a missing workspace would still succeed
/// at build time since restriction is checked per-call, not at construction).
#[test]
fn test_build_agent_no_sandbox_emits_warning() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Config with explicit restrict_to_workspace: true
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}},"agents":{"defaults":{"restrict_to_workspace":true}}}"#,
    )
    .unwrap();

    let flags = AgentFlags {
        session_name: None,
        no_session: false,
        message: Some("hi".into()),
        system_prompt: None,
        model_override: None,
        max_iterations: None,
        max_time: None,
        mode: super::AgentMode::OneShot,
        no_sandbox: true,
    };

    let mut stderr = String::new();
    let result = build_agent_from_config(tmp.path(), &flags, &mut stderr);

    // Agent must build successfully
    assert!(result.is_some(), "stderr: {}", stderr);
    // Warning must be emitted
    assert!(
        stderr.contains("--no-sandbox"),
        "expected --no-sandbox warning in stderr, got: {}",
        stderr
    );
}

/// Verify that without --no-sandbox, config's restrict_to_workspace: true is
/// honoured (no warning emitted, agent still builds).
#[test]
fn test_build_agent_with_sandbox_enabled_no_warning() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();

    let flags = AgentFlags {
        session_name: None,
        no_session: false,
        message: Some("hi".into()),
        system_prompt: None,
        model_override: None,
        max_iterations: None,
        max_time: None,
        mode: super::AgentMode::OneShot,
        no_sandbox: false,
    };

    let mut stderr = String::new();
    let result = build_agent_from_config(tmp.path(), &flags, &mut stderr);
    assert!(result.is_some(), "stderr: {}", stderr);
    assert!(
        !stderr.contains("--no-sandbox"),
        "no warning expected when sandbox is not disabled, got: {}",
        stderr
    );
}
