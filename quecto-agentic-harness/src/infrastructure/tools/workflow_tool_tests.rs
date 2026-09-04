//! Unit tests for the workflow tool (split out of `workflow_tool.rs` to
//! keep the source file within the line-count baseline).

use super::*;
use crate::domain::workflow::{
    WorkflowConfig, WorkflowMode, WorkflowTemplate, WorkflowTemplateStep,
};

fn simple_template(id: &str) -> WorkflowTemplate {
    WorkflowTemplate {
        id: id.into(),
        label: id.into(),
        description: "test template".into(),
        when_to_use: Some("tests".into()),
        steps: vec![
            WorkflowTemplateStep {
                key: "a".into(),
                label: "A".into(),
                phase: "x".into(),
                guidance: Some("first".into()),
            },
            WorkflowTemplateStep {
                key: "b".into(),
                label: "B".into(),
                phase: "x".into(),
                guidance: Some("second".into()),
            },
        ],
        guards: vec![],
    }
}

fn workflow_test_config() -> WorkflowConfig {
    WorkflowConfig {
        templates: vec![simple_template("feature"), simple_template("bugfix")],
        ..WorkflowConfig::default()
    }
}

fn test_tool() -> WorkflowTool {
    let engine = Arc::new(Mutex::new(
        WorkflowEngine::new(workflow_test_config(), true).unwrap(),
    ));
    WorkflowTool::new(engine)
}

#[tokio::test]
async fn status_starts_in_selector_mode() {
    let tool = test_tool();
    let result = tool.execute(r#"{"action":"status"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Template Selection"));
}

#[tokio::test]
async fn list_templates_works() {
    let tool = test_tool();
    let result = tool
        .execute(r#"{"action":"list_templates"}"#)
        .await
        .unwrap();
    assert!(result.content.contains("feature"));
}

#[tokio::test]
async fn select_template_and_check_flow() {
    let tool = test_tool();
    let result = tool
        .execute(r#"{"action":"select_template","template":"feature"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    let result = tool
        .execute(r#"{"action":"check","step":1}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
}

// ─── #1113 cache-safe prompting: guidance travels in tool results ────────

/// Two-step template with known guidance strings for #1113 assertions.
fn guided_template() -> crate::domain::workflow::WorkflowTemplate {
    use crate::domain::workflow::{WorkflowTemplate, WorkflowTemplateStep};
    WorkflowTemplate {
        id: "guided".to_string(),
        label: "Guided".to_string(),
        description: "guided template".to_string(),
        when_to_use: None,
        steps: vec![
            WorkflowTemplateStep {
                key: "first".to_string(),
                label: "First guided step".to_string(),
                phase: "red".to_string(),
                guidance: Some("first step guidance text".to_string()),
            },
            WorkflowTemplateStep {
                key: "second".to_string(),
                label: "Second guided step".to_string(),
                phase: "green".to_string(),
                guidance: Some("second step guidance text".to_string()),
            },
        ],
        guards: vec![],
    }
}

fn guided_tool() -> WorkflowTool {
    let engine = Arc::new(Mutex::new(
        WorkflowEngine::new(
            WorkflowConfig {
                templates: vec![guided_template()],
                ..WorkflowConfig::default()
            },
            true,
        )
        .unwrap(),
    ));
    WorkflowTool::new(engine)
}

/// #1113 AC2: with a static system prompt, select_template must hand the
/// model the current (first) step's label and guidance in its result.
#[tokio::test]
async fn select_template_result_carries_first_step_label_and_guidance() {
    let tool = guided_tool();
    let result = tool
        .execute(r#"{"action":"select_template","template":"guided"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("First guided step"),
        "select_template result must name the current step: {}",
        result.content
    );
    assert!(
        result.content.contains("first step guidance text"),
        "select_template result must carry the current step's guidance: {}",
        result.content
    );
}

/// #1113 AC2: check must hand the model the NEXT step's label and
/// guidance exactly when it advances to it.
#[tokio::test]
async fn check_result_carries_next_step_label_and_guidance() {
    let tool = guided_tool();
    tool.execute(r#"{"action":"select_template","template":"guided"}"#)
        .await
        .unwrap();
    let result = tool
        .execute(r#"{"action":"check","step":1}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("Second guided step"),
        "check result must name the next step: {}",
        result.content
    );
    assert!(
        result.content.contains("second step guidance text"),
        "check result must carry the next step's guidance: {}",
        result.content
    );
}

/// #1113 AC2: skip advances the current step exactly like check, so its
/// result must hand the model the next step's label and guidance.
#[tokio::test]
async fn skip_result_carries_next_step_label_and_guidance() {
    let tool = guided_tool();
    tool.execute(r#"{"action":"select_template","template":"guided"}"#)
        .await
        .unwrap();
    let result = tool.execute(r#"{"action":"skip","step":1}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("Second guided step"),
        "skip result must name the next step: {}",
        result.content
    );
    assert!(
        result.content.contains("second step guidance text"),
        "skip result must carry the next step's guidance: {}",
        result.content
    );
}

/// #1113 AC2: uncheck can move the current step BACKWARDS — its result
/// must re-orient the model on the step the workflow rewound to.
#[tokio::test]
async fn uncheck_result_carries_rewound_current_step_label_and_guidance() {
    let tool = guided_tool();
    tool.execute(r#"{"action":"select_template","template":"guided"}"#)
        .await
        .unwrap();
    tool.execute(r#"{"action":"check","step":1}"#)
        .await
        .unwrap();
    let result = tool
        .execute(r#"{"action":"uncheck","step":1}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("First guided step"),
        "uncheck result must name the rewound current step: {}",
        result.content
    );
    assert!(
        result.content.contains("first step guidance text"),
        "uncheck result must carry the rewound step's guidance: {}",
        result.content
    );
}

/// #1113 AC2: the tool-result handoff is the immediate replacement for the
/// retired per-turn system prompt, which carried progress and the active
/// issue alongside the current step — select_template and check results
/// must carry all three.
#[tokio::test]
async fn select_and_check_results_carry_progress_and_active_issue() {
    let tool = guided_tool();
    let select = tool
        .execute(
            r#"{"action":"select_template","template":"guided","issueNumber":88,"issueTitle":"Guided probe issue"}"#,
        )
        .await
        .unwrap();
    assert!(!select.is_error);
    assert!(
        select.content.contains("Progress: 0/2 steps complete."),
        "select_template result must carry the progress count: {}",
        select.content
    );
    assert!(
        select.content.contains("#88") && select.content.contains("Guided probe issue"),
        "select_template result must carry the active issue: {}",
        select.content
    );

    let check = tool
        .execute(r#"{"action":"check","step":1}"#)
        .await
        .unwrap();
    assert!(!check.is_error);
    assert!(
        check.content.contains("Progress: 1/2 steps complete."),
        "check result must carry the updated progress count: {}",
        check.content
    );
    assert!(
        check.content.contains("#88") && check.content.contains("Guided probe issue"),
        "check result must carry the active issue: {}",
        check.content
    );
}

/// #1113 AC2: with a static system prompt, `status` is the model's
/// re-orientation channel — it must carry the current step's label and
/// guidance for an active workflow.
///
/// NOTE: this is a regression PIN of pre-#1113 `status_text` behavior
/// (the status channel #1113 leans on), not proof of new #1113 work — it
/// passes against the pre-#1113 implementation by design. The
/// falsifiable #1113 coverage lives in the select_template/check/skip/
/// uncheck handoff tests above and the nudge/static-prompt tests.
#[tokio::test]
async fn status_result_carries_current_step_label_and_guidance() {
    let tool = guided_tool();
    tool.execute(r#"{"action":"select_template","template":"guided"}"#)
        .await
        .unwrap();
    let result = tool.execute(r#"{"action":"status"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(
        result.content.contains("First guided step"),
        "status result must name the current step: {}",
        result.content
    );
    assert!(
        result.content.contains("first step guidance text"),
        "status result must carry the current step's guidance: {}",
        result.content
    );
}

/// #1113 AC3: with no selector text injected into the system prompt, the
/// tool's schema description must advertise template discovery/selection.
#[test]
fn tool_description_advertises_template_selection() {
    let definition = test_tool().definition();
    for needle in ["list_templates", "select_template"] {
        assert!(
            definition.description.contains(needle),
            "workflow tool description must advertise '{needle}': {}",
            definition.description
        );
    }
}

#[tokio::test]
async fn no_template_selected_errors_for_check() {
    let tool = test_tool();
    let result = tool
        .execute(r#"{"action":"check","step":1}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("select_template"));
}

#[test]
fn parse_step_accepts_non_ascii_string_value() {
    let val = serde_json::json!("étape 1");
    let result = parse_step(&serde_json::json!({"step": val}));
    // A non-ASCII string is not a valid u32, so it should return a graceful
    // error rather than panicking or corrupting the display string.
    assert!(result.is_err());
}

#[test]
fn parse_issue_number_accepts_non_ascii_string_value() {
    let val = serde_json::json!("numéro 1");
    let result = parse_optional_issue(&serde_json::json!({"issueNumber": val}));
    assert!(result.is_err());
}

#[tokio::test]
async fn select_template_rejects_issue_title_without_issue_number() {
    let tool = test_tool();
    let result = tool
        .execute(r#"{"action":"select_template","template":"feature","issueTitle":"Title only"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("issueTitle requires issueNumber"));
}

#[tokio::test]
async fn set_issue_rejects_issue_title_without_issue_number() {
    let tool = test_tool();
    let result = tool
        .execute(r#"{"action":"set_issue","issueTitle":"Title only"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("issueTitle requires issueNumber"));
}

#[tokio::test]
async fn issue_argument_shapes_keep_existing_behavior() {
    let tool = guided_tool();

    let select_number_and_title = tool
        .execute(
            r#"{"action":"select_template","template":"guided","issueNumber":88,"issueTitle":"Guided probe issue"}"#,
        )
        .await
        .unwrap();
    assert!(!select_number_and_title.is_error);
    assert!(select_number_and_title.content.contains("#88"));
    assert!(
        select_number_and_title
            .content
            .contains("Guided probe issue")
    );

    let set_number_and_title = tool
        .execute(r#"{"action":"set_issue","issueNumber":99,"issueTitle":"Set issue"}"#)
        .await
        .unwrap();
    assert!(!set_number_and_title.is_error);
    assert!(set_number_and_title.content.contains("#99"));
    assert!(set_number_and_title.content.contains("Set issue"));

    let select_number_only = test_tool()
        .execute(r#"{"action":"select_template","template":"feature","issueNumber":99}"#)
        .await
        .unwrap();
    assert!(select_number_only.is_error);
    assert!(
        select_number_only
            .content
            .contains("missing field: issueTitle")
    );

    let set_number_only = tool
        .execute(r#"{"action":"set_issue","issueNumber":99}"#)
        .await
        .unwrap();
    assert!(set_number_only.is_error);
    assert!(
        set_number_only
            .content
            .contains("missing field: issueTitle")
    );

    let select_no_issue = test_tool()
        .execute(r#"{"action":"select_template","template":"feature"}"#)
        .await
        .unwrap();
    assert!(!select_no_issue.is_error);
    assert!(!select_no_issue.content.contains("Active issue:"));

    let set_no_issue = tool.execute(r#"{"action":"set_issue"}"#).await.unwrap();
    assert!(set_no_issue.is_error);
    assert!(set_no_issue.content.contains("missing field: issueNumber"));
}

#[test]
fn snapshot_event_contains_mode() {
    let engine = WorkflowEngine::new(workflow_test_config(), true).unwrap();
    let event = snapshot_to_event(&engine.snapshot(true));
    assert_eq!(event["type"], "workflow_state");
    assert_eq!(
        event["mode"],
        serde_json::json!(WorkflowMode::SelectingTemplate)
    );
}

fn guard_template(id: &str) -> crate::domain::workflow::WorkflowTemplate {
    use crate::domain::workflow::{WorkflowGuardRule, WorkflowTemplate, WorkflowTemplateStep};
    WorkflowTemplate {
        id: id.to_string(),
        label: id.to_string(),
        description: String::new(),
        when_to_use: None,
        steps: vec![WorkflowTemplateStep {
            key: "s1".to_string(),
            label: "S1".to_string(),
            phase: "p".to_string(),
            guidance: None,
        }],
        guards: vec![WorkflowGuardRule {
            commands: vec!["git push".to_string()],
            before_step_key: "s1".to_string(),
            message: "do s1 first".to_string(),
        }],
    }
}

/// #996 item 3 (PR #999 review): `parsed_rules_for` must parse a template's
/// guards once and reuse the `Arc` on subsequent calls for the SAME id, and
/// must rebuild when the active template id changes (no stale-cache bleed).
#[test]
fn parsed_rules_cache_reuses_by_id_and_invalidates_on_change() {
    let engine = Arc::new(Mutex::new(
        WorkflowEngine::new(workflow_test_config(), true).unwrap(),
    ));
    let guard = WorkflowGuard::new(engine);

    let a = guard_template("alpha");
    let first = guard.parsed_rules_for(&a);
    let second = guard.parsed_rules_for(&a);
    assert!(
        Arc::ptr_eq(&first, &second),
        "same template id must return the cached Arc, not re-parse"
    );

    let b = guard_template("beta");
    let third = guard.parsed_rules_for(&b);
    assert!(
        !Arc::ptr_eq(&second, &third),
        "a different template id must rebuild the parsed rules"
    );
    assert_eq!(third[0].before_step_key, "s1");

    // Switching back re-parses (cache holds a single active entry).
    let fourth = guard.parsed_rules_for(&a);
    assert!(!Arc::ptr_eq(&first, &fourth));
}

#[tokio::test]
async fn broadcast_emitter_stamps_identity_and_serializes_line() {
    let (tx, mut rx) = tokio::sync::broadcast::channel(4);
    let emitter = broadcast_emitter(tx, Some("child".into()), Some("parent".into()));
    emitter(serde_json::json!({"type":"workflow_state","mode":"active"}));
    let line = rx.recv().await.unwrap();
    assert!(line.ends_with('\n'));
    let event: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
    assert_eq!(event["type"], "workflow_state");
    assert_eq!(event["agent_id"], "child");
    assert_eq!(event["parent_id"], "parent");
}

#[test]
fn parse_step_rejects_missing_large_and_non_numeric_values() {
    assert_eq!(
        parse_step(&serde_json::json!({})).unwrap_err(),
        "missing field: step"
    );
    assert!(
        parse_step(&serde_json::json!({"step": (u32::MAX as u64) + 1}))
            .unwrap_err()
            .contains("exceeds valid range")
    );
    let err = parse_step(&serde_json::json!({"step": "x".repeat(150)})).unwrap_err();
    assert!(err.starts_with("invalid step value: "));
    assert!(
        err.len() < 130,
        "long invalid values should be truncated: {err}"
    );
}

#[test]
fn parse_issue_rejects_out_of_range_and_non_string_titles() {
    assert!(
        parse_optional_issue(
            &serde_json::json!({"issueNumber": (u32::MAX as u64) + 1, "issueTitle":"T"})
        )
        .unwrap_err()
        .contains("exceeds u32 range")
    );
    assert!(
        parse_optional_issue(&serde_json::json!({"issueNumber": 1, "issueTitle": 5}))
            .unwrap_err()
            .contains("missing field: issueTitle")
    );
    assert_eq!(
        parse_issue(&serde_json::json!({"issueNumber":"42","issueTitle":"Meaning"})).unwrap(),
        (42, "Meaning".to_string())
    );
}

#[tokio::test]
async fn mutating_actions_emit_workflow_events_but_status_does_not() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let tool = WorkflowTool::with_event_emitter(
        Arc::new(Mutex::new(
            WorkflowEngine::new(workflow_test_config(), true).unwrap(),
        )),
        Arc::new(move |event| captured.lock().unwrap().push(event)),
    );
    let status = tool.execute(r#"{"action":"status"}"#).await.unwrap();
    assert!(!status.is_error);
    assert!(events.lock().unwrap().is_empty());
    let selected = tool
        .execute(r#"{"action":"select_template","template":"feature"}"#)
        .await
        .unwrap();
    assert!(!selected.is_error);
    let events = events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "workflow_state");
    assert_eq!(events[0]["activeTemplate"]["id"], "feature");
}
