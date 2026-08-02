//! Region-coverage tests for `parse_agent_flags` / `validate_agent_flags`.
//!
//! Pure flag-parsing logic only — no config load, runtime, or socket I/O.
use super::*;

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

#[test]
fn mode_uds_sets_uds_mode() {
    let mut e = String::new();
    let f = parse_agent_flags(&argv(&["--mode", "uds"]), &mut e).unwrap();
    assert!(f.uds_mode);
}

#[test]
fn mode_invalid_returns_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--mode", "bogus"]), &mut e).is_none());
    assert!(e.contains("not valid"));
}

#[test]
fn mode_missing_value_returns_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--mode"]), &mut e).is_none());
}

#[test]
fn socket_sets_path() {
    let mut e = String::new();
    let f =
        parse_agent_flags(&argv(&["--mode", "uds", "--socket", "/tmp/x.sock"]), &mut e).unwrap();
    assert_eq!(
        f.socket_path.as_deref(),
        Some(std::path::Path::new("/tmp/x.sock"))
    );
}

#[test]
fn socket_missing_value_returns_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--socket"]), &mut e).is_none());
}

#[test]
fn persist_requires_uds() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--persist", "-m", "hi"]), &mut e).is_none());
    assert!(e.contains("--persist requires --mode uds"));
}

#[test]
fn persist_with_uds_ok() {
    let mut e = String::new();
    let f = parse_agent_flags(&argv(&["--mode", "uds", "--persist"]), &mut e).unwrap();
    assert!(f.persist && f.uds_mode);
}

#[test]
fn disable_tool_collects_multiple() {
    let mut e = String::new();
    let f = parse_agent_flags(
        &argv(&["--disable-tool", "bash", "--disable-tool", "edit"]),
        &mut e,
    )
    .unwrap();
    assert_eq!(
        f.disabled_tools,
        vec!["bash".to_string(), "edit".to_string()]
    );
}

#[test]
fn disable_tool_missing_value_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--disable-tool"]), &mut e).is_none());
}

#[test]
fn effort_valid_parses() {
    let mut e = String::new();
    let f = parse_agent_flags(&argv(&["--effort", "high"]), &mut e).unwrap();
    assert!(matches!(
        f.effort,
        Some(crate::domain::provider::EffortLevel::High)
    ));
}

#[test]
fn effort_invalid_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--effort", "ultra"]), &mut e).is_none());
    assert!(e.contains("invalid effort level"));
}

#[test]
fn effort_missing_value_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--effort"]), &mut e).is_none());
}

#[test]
fn config_flag_consumes_value() {
    let mut e = String::new();
    let f = parse_agent_flags(&argv(&["--config", "/tmp/c.toml", "-m", "hi"]), &mut e).unwrap();
    assert_eq!(f.message.as_deref(), Some("hi"));
}

#[test]
fn config_missing_value_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--config"]), &mut e).is_none());
}

#[test]
fn workflow_spec_with_uds_sets_path() {
    let mut e = String::new();
    let f = parse_agent_flags(
        &argv(&["--mode", "uds", "--workflow-spec", "/tmp/w.toml"]),
        &mut e,
    )
    .unwrap();
    assert_eq!(
        f.workflow_spec_path.as_deref(),
        Some(std::path::Path::new("/tmp/w.toml"))
    );
}

#[test]
fn workflow_spec_requires_uds() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--workflow-spec", "/tmp/w.toml"]), &mut e).is_none());
    assert!(e.contains("require --mode uds"));
}

#[test]
fn workflow_spec_conflicts_no_workflow() {
    let mut e = String::new();
    let r = parse_agent_flags(
        &argv(&[
            "--mode",
            "uds",
            "--workflow-spec",
            "/tmp/w.toml",
            "--no-workflow",
        ]),
        &mut e,
    );
    assert!(r.is_none());
    assert!(e.contains("cannot be combined with --no-workflow"));
}

#[test]
fn parent_id_sets_value() {
    let mut e = String::new();
    let f = parse_agent_flags(&argv(&["--parent-id", "root-1"]), &mut e).unwrap();
    assert_eq!(f.parent_id.as_deref(), Some("root-1"));
    // #1319: parent_id alone must not imply spawned.
    assert!(!f.spawned);
}

#[test]
fn parent_id_missing_value_none() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--parent-id"]), &mut e).is_none());
}

/// #1319: internal `--spawned` parses as an explicit flag, independent of parent-id.
#[test]
fn spawned_flag_sets_value() {
    let mut e = String::new();
    let f = parse_agent_flags(&argv(&["--spawned"]), &mut e).unwrap();
    assert!(f.spawned);
    assert!(f.parent_id.is_none());
}

/// #1319: default (top-level) agents are not spawned.
#[test]
fn spawned_defaults_false() {
    let mut e = String::new();
    let f = parse_agent_flags(&argv(&["--mode", "uds"]), &mut e).unwrap();
    assert!(!f.spawned);
}

/// #1319: child argv shape from SpawnTool parses both flags independently.
#[test]
fn spawned_and_parent_id_parse_together() {
    let mut e = String::new();
    let f = parse_agent_flags(
        &argv(&[
            "--mode",
            "uds",
            "-s",
            "child",
            "--persist",
            "--spawned",
            "--parent-id",
            "parent-7",
        ]),
        &mut e,
    )
    .unwrap();
    assert!(f.spawned);
    assert_eq!(f.parent_id.as_deref(), Some("parent-7"));
    assert!(f.uds_mode);
}

#[test]
fn no_session_and_session_mutually_exclusive() {
    let mut e = String::new();
    assert!(parse_agent_flags(&argv(&["--no-session", "-s", "foo"]), &mut e).is_none());
    assert!(e.contains("mutually exclusive"));
}

#[test]
fn no_sandbox_flag_sets() {
    let mut e = String::new();
    let f = parse_agent_flags(&argv(&["--no-sandbox"]), &mut e).unwrap();
    assert!(f.no_sandbox);
}

#[test]
fn workflow_guards_conflicts_no_workflow() {
    let mut e = String::new();
    let r = parse_agent_flags(
        &argv(&["--mode", "uds", "--no-workflow", "--workflow-guards"]),
        &mut e,
    );
    assert!(r.is_none());
    assert!(e.contains("cannot be used with --no-workflow"));
}

#[test]
fn workflow_and_guards_combo_in_uds() {
    let mut e = String::new();
    let f = parse_agent_flags(
        &argv(&["--mode", "uds", "--workflow", "--workflow-guards"]),
        &mut e,
    )
    .unwrap();
    assert!(f.workflow && f.workflow_guards && !f.workflow_disabled);
}

#[test]
fn parse_agent_flags_covers_boolean_bundle_and_missing_values() {
    let mut stderr = String::new();
    let flags = parse_agent_flags(
        &argv(&[
            "--workflow",
            "--workflow-guards",
            "--persist",
            "--mode",
            "uds",
            "--message",
            "hello",
            "--system",
            "sys",
            "--model",
            "local/model",
            "--max-iterations",
            "3",
            "--max-time",
            "4",
            "--socket",
            "/tmp/agent.sock",
            "--disable-tool",
            "bash",
            "--effort",
            "low",
            "--workflow-spec",
            "/tmp/spec.json",
            "--parent-id",
            "parent",
            "--no-sandbox",
            "--no-session",
        ]),
        &mut stderr,
    )
    .unwrap();
    assert!(flags.workflow);
    assert!(flags.workflow_guards);
    assert!(flags.persist);
    assert!(flags.uds_mode);
    assert_eq!(flags.message.as_deref(), Some("hello"));
    assert_eq!(flags.system_prompt.as_deref(), Some("sys"));
    assert_eq!(flags.model_override.as_deref(), Some("local/model"));
    assert_eq!(flags.max_iterations, Some(3));
    assert_eq!(flags.max_time, Some(4));
    assert!(flags.no_sandbox);
    assert!(flags.no_session);
    assert_eq!(flags.disabled_tools, vec!["bash".to_string()]);
    assert!(stderr.is_empty());

    for bad in [
        vec!["--message"],
        vec!["--system"],
        vec!["--model"],
        vec!["--max-iterations", "NaN"],
        vec!["--max-time", "NaN"],
        vec!["--workflow-spec"],
    ] {
        let mut e = String::new();
        assert!(
            parse_agent_flags(&argv(&bad), &mut e).is_none(),
            "bad={bad:?}"
        );
        assert!(!e.is_empty(), "expected diagnostic for {bad:?}");
    }
}

#[test]
fn cmd_agent_uds_rejects_overlong_socket_before_config_load() {
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
        socket_path: Some(std::path::PathBuf::from(format!(
            "/tmp/{}",
            "x".repeat(140)
        ))),
        persist: false,
        disabled_tools: Vec::new(),
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: true,
        workflow_spec_path: None,
        inherited_tool_policy: None,
        parent_id: None,
        spawned: false,
    };
    let ctx = CliContext::default();
    let mut stderr = String::new();
    assert_eq!(cmd_agent_uds(&ctx, flags, &mut stderr), 1);
    assert!(stderr.contains("socket path exceeds"), "{stderr}");

    let mut flags = AgentFlags {
        session_name: None,
        no_session: false,
        message: None,
        system_prompt: None,
        model_override: None,
        max_iterations: None,
        max_time: None,
        uds_mode: true,
        no_sandbox: false,
        socket_path: Some(std::path::PathBuf::from(format!(
            "/tmp/{}",
            "y".repeat(140)
        ))),
        persist: false,
        disabled_tools: Vec::new(),
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: true,
        workflow_spec_path: None,
        inherited_tool_policy: None,
        parent_id: None,
        spawned: false,
    };
    flags.persist = true;
    stderr.clear();
    assert_eq!(cmd_agent_uds(&ctx, flags, &mut stderr), 1);
    assert!(
        !stderr.contains("--persist keeps"),
        "length check should return first: {stderr}"
    );
}
