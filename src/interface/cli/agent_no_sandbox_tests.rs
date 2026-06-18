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
        uds_mode: false,
        no_sandbox: true,
        network: false,
        socket_path: None,
        persist: false,
        disabled_tools: vec![],
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: false,
        workflow_spec_path: None,
        parent_id: None,
    };

    let mut stderr = String::new();
    let result = build_agent_from_config(
        tmp.path(),
        &tmp.path().join("config.json"),
        &flags,
        &mut stderr,
        None,
    );

    // Agent must build successfully
    assert!(result.is_some(), "stderr: {}", stderr);
    // Warning must be emitted
    assert!(
        stderr.contains("--no-sandbox"),
        "expected --no-sandbox warning in stderr, got: {}",
        stderr
    );
}

// ===================================================================
// --no-sandbox uses CWD as workspace root
// ===================================================================

/// `shared::resolve_agent_workspace` must return CWD when no_sandbox=true.
#[test]
fn test_resolve_agent_workspace_no_sandbox_returns_cwd() {
    let cwd = std::env::current_dir().unwrap();
    let result =
        crate::interface::shared::resolve_agent_workspace("/some/configured/workspace", true);
    assert_eq!(
        result, cwd,
        "--no-sandbox should return current_dir(), got {:?}",
        result
    );
}

/// `shared::resolve_agent_workspace` must return the config workspace when no_sandbox=false.
#[test]
fn test_resolve_agent_workspace_sandboxed_returns_config() {
    let result =
        crate::interface::shared::resolve_agent_workspace("/some/configured/workspace", false);
    assert_eq!(
        result.to_string_lossy(),
        "/some/configured/workspace",
        "sandboxed mode should use config workspace"
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
        uds_mode: false,
        no_sandbox: false,
        network: false,
        socket_path: None,
        persist: false,
        disabled_tools: vec![],
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: false,
        workflow_spec_path: None,
        parent_id: None,
    };

    let mut stderr = String::new();
    let result = build_agent_from_config(
        tmp.path(),
        &tmp.path().join("config.json"),
        &flags,
        &mut stderr,
        None,
    );
    assert!(result.is_some(), "stderr: {}", stderr);
    assert!(
        !stderr.contains("--no-sandbox"),
        "no warning expected when sandbox is not disabled, got: {}",
        stderr
    );
}

// ===================================================================
// --network flag parsing tests
// ===================================================================

#[test]
fn test_agent_network_flag_defaults_false() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(!flags.network, "--network should be false by default");
}

#[test]
fn test_agent_network_flag_parsed() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--network".into(), "-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.network, "--network should be true when provided");
}

#[test]
fn test_agent_network_combined_with_no_sandbox() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--network".into(),
        "--no-sandbox".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.network);
    assert!(flags.no_sandbox);
}

#[test]
fn test_agent_network_combined_with_no_session() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--network".into(),
        "--no-session".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.network);
    assert!(flags.no_session);
}

#[test]
fn test_agent_help_documents_network() {
    let out = run_with_output(vec!["quecto".into(), "help".into()], &default_ctx());
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.contains("--network"),
        "help text must document --network, got:\n{}",
        out.stdout
    );
}

// ===================================================================
// --network integration: build_agent_from_config
// ===================================================================

/// --network should override config network_passthrough=false and emit a warning.
#[test]
fn test_build_agent_network_flag_emits_warning() {
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
        uds_mode: false,
        no_sandbox: false,
        network: true,
        socket_path: None,
        persist: false,
        disabled_tools: vec![],
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: false,
        workflow_spec_path: None,
        parent_id: None,
    };

    let mut stderr = String::new();
    let result = build_agent_from_config(
        tmp.path(),
        &tmp.path().join("config.json"),
        &flags,
        &mut stderr,
        None,
    );

    assert!(result.is_some(), "stderr: {}", stderr);
    assert!(
        stderr.contains("--network"),
        "expected --network warning in stderr, got: {}",
        stderr
    );
    assert!(
        stderr.contains("network namespace isolation disabled"),
        "expected network namespace message in stderr, got: {}",
        stderr
    );
}

/// Without --network, no network warning should appear.
#[test]
fn test_build_agent_without_network_flag_no_warning() {
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
        uds_mode: false,
        no_sandbox: false,
        network: false,
        socket_path: None,
        persist: false,
        disabled_tools: vec![],
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: false,
        workflow_spec_path: None,
        parent_id: None,
    };

    let mut stderr = String::new();
    let result = build_agent_from_config(
        tmp.path(),
        &tmp.path().join("config.json"),
        &flags,
        &mut stderr,
        None,
    );
    assert!(result.is_some(), "stderr: {}", stderr);
    assert!(
        !stderr.contains("--network"),
        "no --network warning expected, got: {}",
        stderr
    );
}
