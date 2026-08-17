use super::*;
use crate::domain::tool::{Tool, ToolGuard};
use crate::domain::workflow::{
    WorkflowConfig, WorkflowEngine, WorkflowGuardRule, WorkflowTemplate, WorkflowTemplateStep,
};
use std::sync::{Arc, Mutex};

fn tool_with_config(config: WorkflowConfig, guards_enabled: bool) -> WorkflowTool {
    let engine = Arc::new(Mutex::new(
        WorkflowEngine::new(config, guards_enabled).expect("workflow config should be valid"),
    ));
    WorkflowTool::new(engine)
}

fn default_tool() -> WorkflowTool {
    tool_with_config(WorkflowConfig::default(), true)
}

fn engine_handle_with_config(
    config: WorkflowConfig,
    guards_enabled: bool,
) -> Arc<Mutex<WorkflowEngine>> {
    Arc::new(Mutex::new(
        WorkflowEngine::new(config, guards_enabled).expect("workflow config should be valid"),
    ))
}

fn tool_with_emitter(
    config: WorkflowConfig,
    guards_enabled: bool,
) -> (WorkflowTool, Arc<Mutex<Vec<serde_json::Value>>>) {
    let engine = engine_handle_with_config(config, guards_enabled);
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let emitter: WorkflowEventEmitter = Arc::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });
    (WorkflowTool::with_event_emitter(engine, emitter), events)
}

fn custom_template(
    id: &str,
    label: &str,
    description: &str,
    when_to_use: Option<&str>,
    steps: Vec<WorkflowTemplateStep>,
    guards: Vec<WorkflowGuardRule>,
) -> WorkflowTemplate {
    WorkflowTemplate {
        id: id.into(),
        label: label.into(),
        description: description.into(),
        when_to_use: when_to_use.map(str::to_string),
        steps,
        guards,
    }
}

fn step(key: &str, label: &str, phase: &str) -> WorkflowTemplateStep {
    WorkflowTemplateStep {
        key: key.into(),
        label: label.into(),
        phase: phase.into(),
        guidance: None,
    }
}

fn guarded_config() -> WorkflowConfig {
    WorkflowConfig {
        templates: vec![
            custom_template(
                "guarded",
                "Guarded",
                "Template with a commit guard.",
                Some("Use when guarded commands must wait for planning."),
                vec![
                    step("plan", "Plan work", "red"),
                    step("commit", "Commit", "ci_cd"),
                ],
                vec![WorkflowGuardRule {
                    commands: vec!["git commit".into()],
                    before_step_key: "commit".into(),
                    message: "Finish planning before commit.".into(),
                }],
            ),
            custom_template(
                "open",
                "Open",
                "Template without guards.",
                Some("Use when no guard restrictions apply."),
                vec![
                    step("scope", "Scope task", "red"),
                    step("done", "Finish", "green"),
                ],
                vec![],
            ),
        ],
        ..Default::default()
    }
}

#[tokio::test]
async fn list_templates_includes_when_to_use_metadata() {
    let config = WorkflowConfig {
        templates: vec![custom_template(
            "custom",
            "Custom",
            "Custom template for testing.",
            Some("Use when custom selection guidance matters."),
            vec![step("scope", "Scope", "red")],
            vec![],
        )],
        ..Default::default()
    };
    let tool = tool_with_config(config, false);

    let result = tool
        .execute(r#"{"action":"list_templates"}"#)
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("custom"));
    assert!(result.content.contains("Custom template for testing."));
    assert!(
        result
            .content
            .contains("Use when custom selection guidance matters."),
        "expected when_to_use metadata in list_templates output, got: {}",
        result.content
    );
}

#[tokio::test]
async fn list_templates_omits_when_to_use_line_for_templates_without_it() {
    let config = WorkflowConfig {
        templates: vec![
            custom_template(
                "guided",
                "Guided",
                "Guided template.",
                Some("Use when the model needs extra selection guidance."),
                vec![step("scope", "Scope", "red")],
                vec![],
            ),
            custom_template(
                "plain",
                "Plain",
                "Plain template.",
                None,
                vec![step("scope", "Scope", "red")],
                vec![],
            ),
        ],
        ..Default::default()
    };
    let tool = tool_with_config(config, false);

    let result = tool
        .execute(r#"{"action":"list_templates"}"#)
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(
        result
            .content
            .contains("When to use: Use when the model needs extra selection guidance.")
    );
    assert!(result.content.contains("- plain — Plain: Plain template."));
    assert!(!result.content.contains("When to use: \n"));
}

#[tokio::test]
async fn select_template_with_issue_instantiates_active_run() {
    let tool = default_tool();

    let result = tool
        .execute(
            r#"{"action":"select_template","template":"feature","issueNumber":42,"issueTitle":"Fix login bug"}"#,
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(
        result
            .content
            .contains("Selected workflow template 'feature'")
    );

    let status = tool.execute(r#"{"action":"status"}"#).await.unwrap();
    assert!(status.content.contains("## Active Workflow"));
    assert!(status.content.contains("Template: Feature (feature)"));
    assert!(status.content.contains("Active issue: #42 — Fix login bug"));
    assert!(status.content.contains("CURRENT STEP → 1."));
}

#[tokio::test]
async fn step_actions_fail_clearly_without_selected_template() {
    let tool = default_tool();
    let actions = [
        r#"{"action":"check","step":1}"#,
        r#"{"action":"uncheck","step":1}"#,
        r#"{"action":"skip","step":1}"#,
        r#"{"action":"check_guards","command":"git commit"}"#,
    ];

    for args in actions {
        let result = tool.execute(args).await.unwrap();
        assert!(result.is_error, "expected error for {args}");
        assert!(
            result.content.contains("select_template"),
            "expected clear missing-template guidance for {args}, got: {}",
            result.content
        );
    }
}

#[tokio::test]
async fn check_accepts_string_step_numbers() {
    let tool = default_tool();
    tool.execute(r#"{"action":"select_template","template":"feature"}"#)
        .await
        .unwrap();

    let result = tool
        .execute(r#"{"action":"check","step":"1"}"#)
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("Step 1 checked."));
}

#[tokio::test]
async fn check_rejects_invalid_step_values() {
    let tool = default_tool();
    tool.execute(r#"{"action":"select_template","template":"feature"}"#)
        .await
        .unwrap();

    let invalid = tool
        .execute(r#"{"action":"check","step":"abc"}"#)
        .await
        .unwrap();
    assert!(invalid.is_error);
    assert!(invalid.content.contains("invalid step value"));

    let overflow = tool
        .execute(r#"{"action":"check","step":4294967297}"#)
        .await
        .unwrap();
    assert!(overflow.is_error);
    assert!(overflow.content.contains("exceeds valid range"));

    let missing = tool.execute(r#"{"action":"check"}"#).await.unwrap();
    assert!(missing.is_error);
    assert!(missing.content.contains("missing field: step"));
}

#[tokio::test]
async fn set_issue_accepts_string_issue_numbers() {
    let tool = default_tool();

    let result = tool
        .execute(r#"{"action":"set_issue","issueNumber":"42","issueTitle":"My issue"}"#)
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("#42 — My issue"));
}

#[tokio::test]
async fn set_issue_rejects_invalid_issue_numbers() {
    let tool = default_tool();

    let invalid = tool
        .execute(r#"{"action":"set_issue","issueNumber":"abc","issueTitle":"My issue"}"#)
        .await
        .unwrap();
    assert!(invalid.is_error);
    assert!(invalid.content.contains("invalid issueNumber"));

    let missing = tool
        .execute(r#"{"action":"set_issue","issueTitle":"My issue"}"#)
        .await
        .unwrap();
    assert!(missing.is_error);
    assert!(missing.content.contains("issueTitle requires issueNumber"));
}

#[tokio::test]
async fn check_enforces_ordering_and_skip_bypasses_it() {
    let tool = default_tool();
    tool.execute(r#"{"action":"select_template","template":"feature"}"#)
        .await
        .unwrap();

    let check = tool
        .execute(r#"{"action":"check","step":3}"#)
        .await
        .unwrap();
    assert!(check.is_error);
    assert!(check.content.contains("complete step 1"));

    let skip = tool.execute(r#"{"action":"skip","step":7}"#).await.unwrap();
    assert!(!skip.is_error);
    assert!(skip.content.contains("Step 7 skipped."));

    let status = tool.execute(r#"{"action":"status"}"#).await.unwrap();
    assert!(
        !status.content.contains("[✓] 7. Implement the scoped phase"),
        "status must not expose future skipped steps: {}",
        status.content
    );
    let engine = tool.engine();
    let all_steps = engine.lock().unwrap().all_step_statuses();
    assert!(all_steps[6].done);
}

#[tokio::test]
async fn reset_returns_tool_to_selector_mode() {
    let tool = default_tool();
    tool.execute(r#"{"action":"select_template","template":"feature"}"#)
        .await
        .unwrap();
    tool.execute(r#"{"action":"check","step":1}"#)
        .await
        .unwrap();

    let reset = tool.execute(r#"{"action":"reset"}"#).await.unwrap();
    assert!(!reset.is_error);
    assert!(reset.content.contains("template selection mode"));

    let status = tool.execute(r#"{"action":"status"}"#).await.unwrap();
    assert!(status.content.contains("Workflow Template Selection"));
    assert!(status.content.contains("Available templates"));
}

#[tokio::test]
async fn status_reflects_complete_mode() {
    let tool = default_tool();
    tool.execute(r#"{"action":"select_template","template":"feature"}"#)
        .await
        .unwrap();

    for step in 1..=20 {
        let result = tool
            .execute(&format!(r#"{{"action":"check","step":{step}}}"#))
            .await
            .unwrap();
        assert!(!result.is_error);
    }

    let status = tool.execute(r#"{"action":"status"}"#).await.unwrap();
    assert!(status.content.contains("All workflow steps complete"));
}

#[tokio::test]
async fn mutating_actions_emit_events_but_status_does_not() {
    let (tool, events) = tool_with_emitter(WorkflowConfig::default(), true);

    let status = tool.execute(r#"{"action":"status"}"#).await.unwrap();
    assert!(!status.is_error);
    assert_eq!(events.lock().unwrap().len(), 0);

    let select = tool
        .execute(r#"{"action":"select_template","template":"feature"}"#)
        .await
        .unwrap();
    assert!(!select.is_error);
    {
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "workflow_state");
        assert_eq!(events[0]["activeTemplate"]["id"], "feature");
    }

    let check = tool
        .execute(r#"{"action":"check","step":1}"#)
        .await
        .unwrap();
    assert!(!check.is_error);
    let events = events.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1]["progress"]["done"], 1);
}

#[test]
fn workflow_guard_enforces_bash_command_blocking_on_selected_template() {
    let engine = engine_handle_with_config(guarded_config(), true);
    engine
        .lock()
        .unwrap()
        .select_template("guarded", None)
        .unwrap();
    let guard = WorkflowGuard::new(engine.clone());

    let blocked = guard.check("bash", r#"{"command":"git commit -m wip"}"#);
    assert!(blocked.is_err());
    assert!(
        blocked
            .unwrap_err()
            .contains("BLOCKED: Finish planning before commit.")
    );

    assert!(guard.check("read", r#"{"path":"README.md"}"#).is_ok());

    engine.lock().unwrap().check(1).unwrap();
    assert!(
        guard
            .check("bash", r#"{"command":"git commit -m wip"}"#)
            .is_ok()
    );

    engine
        .lock()
        .unwrap()
        .select_template("open", None)
        .unwrap();
    assert!(
        guard
            .check("bash", r#"{"command":"git commit -m wip"}"#)
            .is_ok()
    );
}

#[test]
fn definition_exposes_only_v2_actions() {
    let tool = default_tool();
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let actions: Vec<&str> = schema["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();

    for expected in [
        "status",
        "list_templates",
        "select_template",
        "check",
        "uncheck",
        "skip",
        "reset",
        "set_issue",
        "clear_issue",
        "check_guards",
    ] {
        assert!(
            actions.contains(&expected),
            "missing action {expected}: {actions:?}"
        );
    }
    assert!(
        !actions.contains(&"check_commit"),
        "legacy V1 action leaked into V2 schema: {actions:?}"
    );
}

#[tokio::test]
async fn check_guards_is_command_scoped_for_multi_guard_templates() {
    let tool = default_tool();
    tool.execute(r#"{"action":"select_template","template":"feature"}"#)
        .await
        .unwrap();
    // Check every step before `commit` so the commit guard is satisfied. The
    // feature template has 11 steps ahead of `commit` (hooks, plan intake,
    // semantic contract, test design/review, RED/GREEN, refactor/harden,
    // local review, verify, and version bump).
    for step in 1..=11 {
        tool.execute(&format!(r#"{{"action":"check","step":{step}}}"#))
            .await
            .unwrap();
    }

    let commit_result = tool
        .execute(r#"{"action":"check_guards","command":"git commit -m test"}"#)
        .await
        .unwrap();
    assert!(!commit_result.is_error, "commit guard should be satisfied");

    let merge_result = tool
        .execute(r#"{"action":"check_guards","command":"git merge master"}"#)
        .await
        .unwrap();
    assert!(merge_result.is_error, "merge guard should remain active");
    assert!(merge_result.content.contains("does not merge"));
}

#[tokio::test]
async fn check_guards_only_uses_the_selected_template() {
    let tool = tool_with_config(guarded_config(), true);

    tool.execute(r#"{"action":"select_template","template":"open"}"#)
        .await
        .unwrap();
    let open_result = tool
        .execute(r#"{"action":"check_guards","command":"git commit"}"#)
        .await
        .unwrap();
    assert!(!open_result.is_error);
    assert!(open_result.content.contains("satisfied"));

    tool.execute(r#"{"action":"select_template","template":"guarded"}"#)
        .await
        .unwrap();
    let guarded_result = tool
        .execute(r#"{"action":"check_guards","command":"git commit"}"#)
        .await
        .unwrap();
    assert!(guarded_result.is_error);
    assert!(
        guarded_result
            .content
            .contains("Finish planning before commit.")
    );
}

/// Verify that a broadcast_tx-backed emitter delivers properly formatted
/// workflow_state JSON lines through the tokio broadcast channel (#598).
#[tokio::test]
async fn emitter_sends_workflow_state_through_broadcast_channel() {
    let (broadcast_tx, mut broadcast_rx) = tokio::sync::broadcast::channel::<String>(16);
    let emitter = broadcast_emitter(broadcast_tx.clone(), None, None);
    let engine = engine_handle_with_config(WorkflowConfig::default(), false);
    let tool = WorkflowTool::with_event_emitter(engine, emitter);

    // select_template should send an event through the channel.
    tool.execute(r#"{"action":"select_template","template":"feature"}"#)
        .await
        .unwrap();

    let line = broadcast_rx
        .try_recv()
        .expect("expected a broadcast message");
    assert!(line.ends_with('\n'), "line should end with newline");
    let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(parsed["type"], "workflow_state");
    assert_eq!(parsed["mode"], "active");
    assert_eq!(parsed["activeTemplate"]["id"], "feature");

    // status should NOT send an event.
    tool.execute(r#"{"action":"status"}"#).await.unwrap();
    assert!(
        broadcast_rx.try_recv().is_err(),
        "status should not produce a broadcast event"
    );

    // check should send an event with progress.
    tool.execute(r#"{"action":"check","step":1}"#)
        .await
        .unwrap();
    let line2 = broadcast_rx.try_recv().expect("expected check broadcast");
    let parsed2: serde_json::Value = serde_json::from_str(line2.trim()).unwrap();
    assert_eq!(parsed2["type"], "workflow_state");
    assert_eq!(parsed2["progress"]["done"], 1);
}

/// Verify that register_workflow_tool with a broadcast_tx-backed emitter
/// produces a tool that sends events on the channel (#598).
#[tokio::test]
async fn register_workflow_tool_with_broadcast_emitter() {
    use crate::infrastructure::security::sandbox::Sandbox;

    let (broadcast_tx, mut broadcast_rx) = tokio::sync::broadcast::channel::<String>(16);
    let emitter = broadcast_emitter(broadcast_tx.clone(), None, None);

    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().to_path_buf();
    let sandbox = Sandbox::new(Some(workspace.clone()));
    let mut registry = crate::infrastructure::extensions::native::build_official_tool_registry(
        workspace,
        sandbox,
        Default::default(),
    );

    let _engine = crate::interface::shared::register_workflow_tool(
        &mut registry,
        WorkflowConfig::default(),
        false,
        Some(emitter),
    )
    .expect("register should succeed");

    // Find and execute the workflow tool through the registry.
    let tool = registry
        .get("workflow")
        .expect("workflow tool should be registered");
    let result = tool
        .execute(r#"{"action":"select_template","template":"feature"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);

    let line = broadcast_rx
        .try_recv()
        .expect("expected broadcast after select_template");
    let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(parsed["type"], "workflow_state");
    assert_eq!(parsed["activeTemplate"]["id"], "feature");
}

#[test]
fn broadcast_emitter_stamps_agent_and_parent_identity() {
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(4);
    let emitter = broadcast_emitter(tx, Some("root".to_string()), None);
    emitter(serde_json::json!({ "type": "workflow_state" }));
    let line = rx.try_recv().expect("event line");
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["agent_id"], "root");
    assert!(
        v.as_object().unwrap().contains_key("parent_id"),
        "parent_id field must be present (null at root)"
    );
}

#[test]
fn broadcast_emitter_passes_through_non_object_event() {
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(4);
    let emitter = broadcast_emitter(tx, Some("a".to_string()), Some("b".to_string()));
    // A non-object event has nowhere to stamp identity; it is still forwarded.
    emitter(serde_json::json!("just a string"));
    assert_eq!(rx.try_recv().unwrap().trim(), "\"just a string\"");
}
