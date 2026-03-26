use super::*;
use crate::domain::session::Session;

use crate::interface::cli::{CliContext, CliOutput, run_with_output};

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

/// Helper to check if a message appears in either stdout or stderr.
fn stderr_or_stdout_contains(out: &CliOutput, needle: &str) -> bool {
    out.stderr.contains(needle) || out.stdout.contains(needle)
}

#[test]
fn test_agent_no_message_requires_m_flag() {
    // Without -m, agent should require non-interactive mode
    let out = run_with_output(args("agent"), &default_ctx());
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("-m is required"));
}

#[test]
fn test_agent_session_flag_missing_value() {
    let out = run_with_output(args("agent -s"), &default_ctx());
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("-s requires a session name"));
}

#[test]
fn test_agent_message_flag_missing_value() {
    let out = run_with_output(args("agent -m"), &default_ctx());
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("-m requires a message"));
}

// ===================================================================
// Agent headless one-shot mode tests
// ===================================================================

#[test]
fn test_agent_no_message_shows_usage_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Write a config so it doesn't fail on "config not found"
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("agent"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr
            .contains("agent: -m is required for non-interactive mode"),
        "expected usage error, got stderr: {}",
        out.stderr
    );
}

#[test]
fn test_agent_missing_config_shows_instructions() {
    let tmp = tempfile::TempDir::new().unwrap();
    // No config file written
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("agent -m hello"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("config not found"),
        "expected 'config not found', got stderr: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("quecto onboard"),
        "expected 'quecto onboard', got stderr: {}",
        out.stderr
    );
}

#[test]
fn test_agent_no_providers_shows_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":""},"anthropic":{"api_key":""}}}"#,
    )
    .unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("agent -m hello"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("no LLM providers"),
        "expected 'no LLM providers', got stderr: {}",
        out.stderr
    );
}

#[test]
fn test_agent_parses_system_flag() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(
        vec![
            "quecto".into(),
            "agent".into(),
            "--system".into(),
            "You are a pirate".into(),
            "-m".into(),
            "Hello".into(),
        ],
        &ctx,
    );
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("config not found"));
}

#[test]
fn test_agent_parses_model_flag() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(
        vec![
            "quecto".into(),
            "agent".into(),
            "--model".into(),
            "gpt-5-mini".into(),
            "-m".into(),
            "Hello".into(),
        ],
        &ctx,
    );
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("config not found"));
}

#[test]
fn test_agent_system_flag_missing_value() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("config.json"), "{}").unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("agent --system"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("--system requires a value"));
}

#[test]
fn test_agent_model_flag_missing_value() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("config.json"), "{}").unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("agent --model"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("--model requires a value"));
}

#[test]
fn test_agent_session_flag_parses_name() {
    let mut stderr = String::new();
    let a = vec!["-s".into(), "my-chat".into(), "-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.session_name.as_deref(), Some("my-chat"));
}

#[test]
fn test_agent_session_flag_ephemeral() {
    let mut stderr = String::new();
    let a = vec!["-s".into(), "-".into(), "-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.session_name.as_deref(), Some("-"));
}

#[test]
fn test_agent_session_flag_default_when_absent() {
    let mut stderr = String::new();
    let a = vec!["-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.session_name.is_none());
}

#[test]
fn test_agent_session_key_derivation() {
    assert_eq!(Session::build_key("cli", "default"), "cli:default");
    assert_eq!(Session::build_key("cli", "my-chat"), "cli:my-chat");
}

#[test]
fn test_agent_rejects_path_traversal_session_name() {
    let mut stderr = String::new();
    let a = vec![
        "-s".into(),
        "../../tmp/evil".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let result = parse_agent_flags(&a, &mut stderr);
    assert!(result.is_none());
    assert!(stderr.contains("alphanumeric"));
}

#[test]
fn test_agent_parses_max_iterations_flag() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--max-iterations".into(),
        "5".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.max_iterations, Some(5));
}

#[test]
fn test_agent_parses_max_time_flag() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--max-time".into(), "30".into(), "-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.max_time, Some(30));
}

#[test]
fn test_agent_max_iterations_errors() {
    let mut e = String::new();
    assert!(parse_agent_flags(&["--max-iterations".into()], &mut e).is_none());
    assert!(e.contains("requires a value"));
    e.clear();
    assert!(parse_agent_flags(&["--max-iterations".into(), "abc".into()], &mut e).is_none());
    assert!(e.contains("positive integer"));
    e.clear();
    assert!(parse_agent_flags(&["--max-iterations".into(), "0".into()], &mut e).is_none());
    assert!(e.contains("positive integer"));
}

#[test]
fn test_agent_max_time_errors() {
    let mut e = String::new();
    assert!(parse_agent_flags(&["--max-time".into()], &mut e).is_none());
    assert!(e.contains("requires a value"));
    e.clear();
    assert!(parse_agent_flags(&["--max-time".into(), "xyz".into()], &mut e).is_none());
    assert!(e.contains("positive integer"));
    e.clear();
    assert!(parse_agent_flags(&["--max-time".into(), "0".into()], &mut e).is_none());
    assert!(e.contains("positive integer"));
}

#[test]
fn test_agent_max_iterations_absent_is_none() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.max_iterations.is_none());
    assert!(flags.max_time.is_none());
}

// ===================================================================
// cmd_agent edge cases
// ===================================================================

#[test]
fn test_agent_message_long_flag() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--message".into(), "Hello world".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.message.as_deref(), Some("Hello world"));
}

#[test]
fn test_agent_message_missing_value() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--message".into()];
    let result = parse_agent_flags(&a, &mut stderr);
    assert!(result.is_none());
    assert!(stderr.contains("-m requires a message"));
}

#[test]
fn test_agent_session_long_flag() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--session".into(),
        "my-chat".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.session_name.as_deref(), Some("my-chat"));
}

#[test]
fn test_agent_session_missing_value() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--session".into()];
    let result = parse_agent_flags(&a, &mut stderr);
    assert!(result.is_none());
    assert!(stderr.contains("-s requires a session name"));
}

#[test]
fn test_agent_all_flags_combined() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "-s".into(),
        "sess1".into(),
        "-m".into(),
        "Do something".into(),
        "--system".into(),
        "Be helpful".into(),
        "--model".into(),
        "gpt-4o".into(),
        "--max-iterations".into(),
        "10".into(),
        "--max-time".into(),
        "60".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.session_name.as_deref(), Some("sess1"));
    assert_eq!(flags.message.as_deref(), Some("Do something"));
    assert_eq!(flags.system_prompt.as_deref(), Some("Be helpful"));
    assert_eq!(flags.model_override.as_deref(), Some("gpt-4o"));
    assert_eq!(flags.max_iterations, Some(10));
    assert_eq!(flags.max_time, Some(60));
}

#[test]
fn test_agent_unknown_args_are_skipped() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["positional".into(), "-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.message.as_deref(), Some("Hi"));
    assert!(stderr.is_empty());
}

#[test]
fn test_agent_negative_max_iterations() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--max-iterations".into(), "-1".into()];
    let result = parse_agent_flags(&a, &mut stderr);
    assert!(result.is_none());
    assert!(stderr.contains("positive integer"));
}

#[test]
fn test_agent_negative_max_time() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--max-time".into(), "-5".into()];
    let result = parse_agent_flags(&a, &mut stderr);
    assert!(result.is_none());
    assert!(stderr.contains("positive integer"));
}

#[test]
fn test_agent_workflow_and_no_workflow_flags_require_uds_mode() {
    for args in [
        vec!["--workflow".into(), "-m".into(), "Hi".into()],
        vec!["--workflow-guards".into(), "-m".into(), "Hi".into()],
        vec!["--no-workflow".into(), "-m".into(), "Hi".into()],
    ] {
        let mut stderr = String::new();
        let result = parse_agent_flags(&args, &mut stderr);
        assert!(
            result.is_none(),
            "expected parse failure for args: {args:?}"
        );
        assert!(
            stderr.contains("require --mode uds"),
            "expected UDS validation error for args {args:?}, got: {stderr}"
        );
    }
}

#[test]
fn test_agent_workflow_guards_require_workflow_even_in_uds_mode() {
    let mut stderr = String::new();
    let args: Vec<String> = vec!["--mode".into(), "uds".into(), "--workflow-guards".into()];
    let result = parse_agent_flags(&args, &mut stderr);
    assert!(result.is_none());
    assert!(stderr.contains("--workflow-guards requires --workflow"));
}

#[test]
fn test_agent_no_workflow_clears_workflow_flags_in_uds_mode() {
    let mut stderr = String::new();
    let args: Vec<String> = vec![
        "--mode".into(),
        "uds".into(),
        "--workflow".into(),
        "--workflow-guards".into(),
        "--no-workflow".into(),
    ];
    let flags = parse_agent_flags(&args, &mut stderr).unwrap();
    assert!(!flags.workflow);
    assert!(!flags.workflow_guards);
}

// ===================================================================
// build_agent_from_config tests
// ===================================================================

#[test]
fn test_build_agent_from_config_no_config_file() {
    let tmp = tempfile::TempDir::new().unwrap();
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
    };
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, &flags, &mut stderr, None);
    assert!(result.is_none());
    assert!(stderr.contains("config not found"));
}

#[test]
fn test_build_agent_from_config_invalid_json() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("config.json"), "not json at all").unwrap();
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
    };
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, &flags, &mut stderr, None);
    assert!(result.is_none());
    assert!(stderr.contains("failed to load config"));
}

#[test]
fn test_build_agent_from_config_no_providers() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":""},"anthropic":{"api_key":""}}}"#,
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
    };
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, &flags, &mut stderr, None);
    assert!(result.is_none());
    assert!(stderr.contains("no LLM providers"));
}

#[test]
fn test_build_agent_from_config_with_model_override() {
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
        model_override: Some("gpt-custom".into()),
        max_iterations: Some(5),
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
    };
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, &flags, &mut stderr, None);
    assert!(result.is_some(), "stderr: {}", stderr);
}

#[test]
fn test_build_agent_from_config_workflow_disabled_by_default_has_no_workflow_state() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();
    let flags = AgentFlags {
        session_name: None,
        no_session: false,
        message: None,
        system_prompt: None,
        model_override: None,
        max_iterations: None,
        max_time: None,
        uds_mode: true,
        no_sandbox: false,
        network: false,
        socket_path: None,
        persist: false,
        disabled_tools: vec![],
        effort: None,
        workflow: false,
        workflow_guards: false,
    };
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, &flags, &mut stderr, None)
        .expect("agent should build without workflow enabled");
    assert!(result.workflow_state.is_none(), "stderr: {}", stderr);
    assert!(result.workflow_config.is_none(), "stderr: {}", stderr);
}

#[test]
fn test_build_agent_from_config_uds_workflow_flag_creates_workflow_state() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();
    let flags = AgentFlags {
        session_name: None,
        no_session: false,
        message: None,
        system_prompt: None,
        model_override: None,
        max_iterations: None,
        max_time: None,
        uds_mode: true,
        no_sandbox: false,
        network: false,
        socket_path: None,
        persist: false,
        disabled_tools: vec![],
        effort: None,
        workflow: true,
        workflow_guards: false,
    };
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, &flags, &mut stderr, None)
        .expect("agent should build with workflow enabled");
    assert!(result.workflow_state.is_some(), "stderr: {}", stderr);
    assert!(result.workflow_config.is_some(), "stderr: {}", stderr);
}

// ===================================================================
// Agent cmd_agent integration through run_with_output
// ===================================================================

#[test]
fn test_agent_with_system_and_model_no_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let v = vec![
        "quecto".into(),
        "agent".into(),
        "--system".into(),
        "Be nice".into(),
        "--model".into(),
        "gpt-4o".into(),
        "--max-iterations".into(),
        "3".into(),
        "--max-time".into(),
        "10".into(),
        "-m".into(),
        "Hello".into(),
    ];
    let out = run_with_output(v, &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(stderr_or_stdout_contains(&out, "config not found"));
}

#[test]
fn test_agent_run_with_output_rejects_no_workflow_without_uds_mode() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("agent --no-workflow -m hello"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("require --mode uds"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn test_agent_with_ephemeral_session_no_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let v = vec![
        "quecto".into(),
        "agent".into(),
        "-s".into(),
        "-".into(),
        "-m".into(),
        "Hello".into(),
    ];
    let out = run_with_output(v, &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(stderr_or_stdout_contains(&out, "config not found"));
}

// ===================================================================
// Agent max-iterations/max-time large values
// ===================================================================

#[test]
fn test_agent_large_max_iterations() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--max-iterations".into(),
        "999999".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.max_iterations, Some(999999));
}

#[test]
fn test_agent_large_max_time() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--max-time".into(),
        "86400".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.max_time, Some(86400));
}

#[test]
fn test_agent_max_iterations_overflow_rejected() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--max-iterations".into(), "99999999999999999".into()];
    let result = parse_agent_flags(&a, &mut stderr);
    assert!(result.is_none());
    assert!(stderr.contains("positive integer"));
}

// ===================================================================
// SpawnTool presence in headless agent registry
// ===================================================================

#[test]
fn test_headless_agent_registry_includes_spawn_tool() {
    use crate::infrastructure::config::Config;
    use crate::infrastructure::security::sandbox::Sandbox;
    use crate::infrastructure::tools::registry::ToolRegistryImpl;

    let tmp = tempfile::TempDir::new().unwrap();
    let config_json = r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#;
    std::fs::write(tmp.path().join("config.json"), config_json).unwrap();

    let config = Config::load(tmp.path().join("config.json").to_str().unwrap()).unwrap();
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let sandbox = Sandbox::new(
        Some(workspace.clone()),
        config.agents.defaults.restrict_to_workspace,
    );
    let exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(&config);
    let mut registry = ToolRegistryImpl::with_core_tools_and_exec_settings(
        workspace.clone(),
        sandbox,
        exec_settings,
    );

    // Match what build_agent_from_config does: with_base_dir so spawn is
    // not in stub mode and will actually launch subagents.
    use crate::infrastructure::tools::spawn::SpawnTool;
    use std::sync::Arc;
    let spawn = SpawnTool::with_base_dir(
        vec![],
        config.agents.defaults.restrict_to_workspace,
        tmp.path().to_path_buf(),
    );

    // Assert the constructed tool has a real base_dir (not stub mode).
    let debug_str = format!("{:?}", spawn);
    assert!(
        debug_str.contains(tmp.path().to_str().unwrap()),
        "SpawnTool must be constructed with a real base_dir, got: {}",
        debug_str
    );

    registry.register(Arc::new(spawn));

    let names = registry.names();
    assert!(
        names.contains(&"spawn".to_string()),
        "headless agent registry must include 'spawn' tool, got: {:?}",
        names
    );
}

// --disable-tool flag (#402)

#[test]
fn test_agent_disable_tool_flag_single() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--disable-tool".into(),
        "bash".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.disabled_tools, vec!["bash"]);
}

#[test]
fn test_agent_disable_tool_flag_multiple() {
    let mut stderr = String::new();
    let a: Vec<String> = vec![
        "--disable-tool".into(),
        "bash".into(),
        "--disable-tool".into(),
        "web_fetch".into(),
        "-m".into(),
        "Hi".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(flags.disabled_tools, vec!["bash", "web_fetch"]);
}

#[test]
fn test_agent_disable_tool_flag_missing_value() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["--disable-tool".into()];
    let result = parse_agent_flags(&a, &mut stderr);
    assert!(result.is_none());
    assert!(stderr.contains("--disable-tool requires a tool name"));
}

#[test]
fn test_agent_disable_tool_absent_is_empty() {
    let mut stderr = String::new();
    let a: Vec<String> = vec!["-m".into(), "Hi".into()];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert!(flags.disabled_tools.is_empty());
}

// --persist flag (#348)

#[test]
fn test_agent_persist_flag() {
    let mut e = String::new();
    // Default: persist is false
    let a = vec!["--mode".into(), "uds".into()];
    assert!(!parse_agent_flags(&a, &mut e).unwrap().persist);
    // Explicit --persist sets it to true, combines with other flags
    let a = vec![
        "--mode".into(),
        "uds".into(),
        "--persist".into(),
        "--no-session".into(),
        "--socket".into(),
        "/tmp/t.sock".into(),
    ];
    let f = parse_agent_flags(&a, &mut e).unwrap();
    assert!(f.persist && f.no_session && f.uds_mode);
    assert_eq!(
        f.socket_path.as_deref(),
        Some(std::path::Path::new("/tmp/t.sock"))
    );
    // --persist without --mode uds is rejected
    e.clear();
    let a = vec!["--persist".into(), "-m".into(), "hi".into()];
    assert!(parse_agent_flags(&a, &mut e).is_none());
    assert!(e.contains("--persist requires --mode uds"));
}
