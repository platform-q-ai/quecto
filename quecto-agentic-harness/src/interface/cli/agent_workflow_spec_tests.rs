//! Tests for workflow availability/assignment via the agent CLI:
//! the `--workflow` / `--no-workflow` build paths and the by-value
//! `--workflow-spec` binding (PRD Stage A).

use super::*;

fn uds_workflow_flags(workflow: bool, workflow_disabled: bool) -> AgentFlags {
    AgentFlags {
        session_name: None,
        no_session: false,
        message: None,
        system_prompt: None,
        model_override: None,
        max_iterations: None,
        max_time: None,
        uds_mode: true,
        no_sandbox: false,
        socket_path: None,
        persist: false,
        disabled_tools: vec![],
        effort: None,
        workflow,
        workflow_guards: false,
        workflow_disabled,
        workflow_spec_path: None,
        inherited_tool_policy: None,
        parent_id: None,
        spawned: false,
    }
}

#[test]
fn test_build_agent_from_config_uds_default_makes_workflow_available_without_forced_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();
    let flags = uds_workflow_flags(false, false);
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, false, &flags, &mut stderr, None)
        .expect("agent should build with optional workflow available");
    assert!(result.workflow_state.is_some(), "stderr: {}", stderr);
    assert!(result.workflow_config.is_some(), "stderr: {}", stderr);
    // #1113: without --workflow the selector nudge stays disarmed — a plain
    // UDS session must never be nudged to pick a template at idle boundaries.
    let engine = result.workflow_state.as_ref().unwrap().lock().unwrap();
    assert!(
        engine.auto_continue_nudge().is_none(),
        "normal UDS must not push the template selector at idle boundaries"
    );
}

#[test]
fn test_build_agent_from_config_uds_no_workflow_disables_workflow_state() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();
    let flags = uds_workflow_flags(false, true);
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, false, &flags, &mut stderr, None)
        .expect("agent should build with workflow disabled");
    assert!(result.workflow_state.is_none(), "stderr: {}", stderr);
    assert!(result.workflow_config.is_none(), "stderr: {}", stderr);
    let tool_definitions = result.agent.tool_definitions();
    assert!(
        tool_definitions
            .iter()
            .all(|definition| definition.name.as_ref() != "workflow"),
        "--no-workflow must remove the workflow tool definition sent to the model"
    );
    let spawn_definition = tool_definitions
        .iter()
        .find(|definition| definition.name.as_ref() == "spawn")
        .expect("spawn tool should remain available when parent workflow is disabled");
    assert!(
        spawn_definition.parameters_schema.contains("\"workflow\""),
        "spawn must still allow explicitly workflow-enabled child agents"
    );
}

#[test]
fn test_build_agent_from_config_uds_workflow_flag_creates_workflow_state() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();
    let flags = uds_workflow_flags(true, false);
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, false, &flags, &mut stderr, None)
        .expect("agent should build with workflow enabled");
    assert!(result.workflow_state.is_some(), "stderr: {}", stderr);
    assert!(result.workflow_config.is_some(), "stderr: {}", stderr);
    // #1113: --workflow arms the idle-boundary template selector nudge (no
    // system-prompt injection): the engine yields a selector nudge while no
    // template is selected.
    let engine = result.workflow_state.as_ref().unwrap().lock().unwrap();
    let nudge = engine
        .auto_continue_nudge()
        .expect("--workflow must arm the idle-boundary template selector nudge");
    assert!(
        nudge.contains("select_template"),
        "selector nudge must instruct selection via select_template: {nudge}"
    );
}

// ── by-value workflow spec assignment (CLI flag) ──────────────────────────

#[test]
fn workflow_spec_flag_parses_path_in_uds_mode() {
    let mut stderr = String::new();
    let a = vec![
        "--mode".into(),
        "uds".into(),
        "--workflow-spec".into(),
        "/tmp/spec.json".into(),
    ];
    let flags = parse_agent_flags(&a, &mut stderr).unwrap();
    assert_eq!(
        flags.workflow_spec_path.as_deref(),
        Some(std::path::Path::new("/tmp/spec.json"))
    );
}

#[test]
fn workflow_spec_flag_requires_uds_mode() {
    let mut stderr = String::new();
    let a = vec!["--workflow-spec".into(), "/tmp/spec.json".into()];
    assert!(parse_agent_flags(&a, &mut stderr).is_none());
    assert!(stderr.contains("--mode uds"));
}

#[test]
fn workflow_spec_flag_conflicts_with_no_workflow() {
    let mut stderr = String::new();
    let a = vec![
        "--mode".into(),
        "uds".into(),
        "--workflow-spec".into(),
        "/tmp/spec.json".into(),
        "--no-workflow".into(),
    ];
    assert!(parse_agent_flags(&a, &mut stderr).is_none());
    assert!(stderr.contains("cannot be combined with --no-workflow"));
}

// ── by-value workflow spec binding (end-to-end through build) ─────────────

#[test]
fn build_agent_with_workflow_spec_binds_template_in_active_mode() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();
    // A by-value spec carrying a single inline template.
    let spec_path = tmp.path().join("spec.json");
    std::fs::write(
        &spec_path,
        r#"{"template":{"id":"review-pr","label":"Review PR","description":"d","steps":[{"key":"analyze","label":"Analyze","phase":"review"}]}}"#,
    )
    .unwrap();

    let mut flags = uds_workflow_flags(false, false);
    flags.workflow_spec_path = Some(spec_path);
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, false, &flags, &mut stderr, None)
        .expect("agent should build with a bound workflow spec");

    let handle = result
        .workflow_state
        .expect("workflow state should exist for a bound spec");
    let engine = handle.lock().unwrap();
    // Bound: exactly the assigned template, pre-selected into Active mode, and
    // the engine is locked to it.
    assert_eq!(engine.mode(), crate::domain::workflow::WorkflowMode::Active);
    assert!(
        engine.is_bound(),
        "engine should be bound to the assigned template"
    );
    let templates = engine.list_templates();
    assert_eq!(templates.len(), 1, "stderr: {stderr}");
    assert_eq!(templates[0].id, "review-pr");
}

#[test]
fn build_agent_with_unloadable_workflow_spec_fails_closed() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();
    // Point at a spec file that does not exist.
    let mut flags = uds_workflow_flags(false, false);
    flags.workflow_spec_path = Some(tmp.path().join("missing.json"));
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, false, &flags, &mut stderr, None)
        .expect("agent should still build");
    // Fail closed: an assigned-but-unloadable spec must NOT degrade into a
    // free-selection workflow agent — no workflow is registered.
    assert!(result.workflow_state.is_none(), "stderr: {stderr}");
    assert!(
        stderr.contains("refusing to start a workflow"),
        "stderr: {stderr}"
    );
}

#[test]
fn build_agent_with_semantically_invalid_workflow_spec_fails_startup() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();
    let spec_path = tmp.path().join("spec.json");
    std::fs::write(
        &spec_path,
        r#"{"template":{"id":"review-pr","label":"Review PR","description":"d","steps":[{"key":" ","label":"Analyze","phase":"review"}]}}"#,
    )
    .unwrap();

    let mut flags = uds_workflow_flags(false, false);
    flags.workflow_spec_path = Some(spec_path);
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, false, &flags, &mut stderr, None);

    assert!(result.is_none(), "an invalid bound spec must abort startup");
    assert!(stderr.contains("failed to initialize workflow"), "{stderr}");
    assert!(stderr.contains("empty key"), "{stderr}");
}

#[test]
fn build_agent_with_oversized_workflow_spec_fails_closed() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();
    // A spec larger than MAX_WORKFLOW_SPEC_BYTES must be rejected on load.
    let filler = "z".repeat(crate::domain::workflow::MAX_WORKFLOW_SPEC_BYTES + 1);
    let huge = format!(
        r#"{{"template":{{"id":"x","label":"x","description":"{filler}","steps":[{{"key":"a","label":"A","phase":"p"}}]}}}}"#
    );
    let spec_path = tmp.path().join("spec.json");
    std::fs::write(&spec_path, huge).unwrap();
    let mut flags = uds_workflow_flags(false, false);
    flags.workflow_spec_path = Some(spec_path);
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, false, &flags, &mut stderr, None)
        .expect("agent should still build");
    assert!(result.workflow_state.is_none(), "stderr: {stderr}");
    assert!(stderr.contains("too large"), "stderr: {stderr}");
}
