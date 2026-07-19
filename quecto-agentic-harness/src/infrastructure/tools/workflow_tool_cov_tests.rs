use super::*;
use crate::domain::tool::{Tool, ToolGuard};
use crate::domain::workflow::{
    WorkflowConfig, WorkflowEngine, WorkflowGuardRule, WorkflowTemplate, WorkflowTemplateStep,
};
use std::sync::{Arc, Mutex};

fn guarded_template(id: &str) -> WorkflowTemplate {
    WorkflowTemplate {
        id: id.to_string(),
        label: format!("{id} label"),
        description: "desc".into(),
        when_to_use: Some("tests".into()),
        steps: vec![
            WorkflowTemplateStep {
                key: "plan".into(),
                label: "Plan".into(),
                phase: "prep".into(),
                guidance: Some("write a plan".into()),
            },
            WorkflowTemplateStep {
                key: "test".into(),
                label: "Test".into(),
                phase: "verify".into(),
                guidance: None,
            },
        ],
        guards: vec![WorkflowGuardRule {
            commands: vec!["cargo test".into()],
            before_step_key: "test".into(),
            message: "planning must be complete".into(),
        }],
    }
}

fn engine_handle() -> WorkflowEngineHandle {
    let config = WorkflowConfig {
        templates: vec![guarded_template("feature"), guarded_template("refactor")],
        ..WorkflowConfig::default()
    };
    Arc::new(Mutex::new(WorkflowEngine::new(config, true).unwrap()))
}

#[test]
fn workflow_tool_debug_engine_and_poisoned_lock_paths() {
    let handle = engine_handle();
    let tool = WorkflowTool::new(handle.clone());
    assert_eq!(tool.engine().lock().unwrap().list_templates().len(), 2);

    let poisoned = handle.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoned.lock().unwrap();
        panic!("poison workflow engine for coverage");
    })
    .join();

    let err = tool.lock_engine().unwrap_err();
    assert!(err.contains("workflow engine poisoned"));
}

#[tokio::test]
async fn handle_action_selects_checks_lists_and_emits_events() {
    let handle = engine_handle();
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let tool = WorkflowTool::with_event_emitter(
        handle,
        Arc::new(move |event| seen.lock().unwrap().push(event)),
    );

    let listed = tool
        .handle_action(r#"{"action":"list_templates"}"#)
        .unwrap();
    assert!(listed.contains("feature label"));
    assert!(listed.contains("When to use: tests"));

    let selected = tool
        .handle_action(
            r#"{"action":"select_template","template":"feature","issueNumber":"42","issueTitle":"ship it"}"#,
        )
        .unwrap();
    assert!(selected.contains("Selected workflow template 'feature'"));
    assert!(selected.contains("Current step"));

    let blocked = tool
        .handle_action(r#"{"action":"check_guards","command":"cargo test -p quecto"}"#)
        .unwrap_err();
    assert!(blocked.contains("planning must be complete"));

    let checked = tool
        .handle_action(r#"{"action":"check","step":1}"#)
        .unwrap();
    assert!(checked.contains("Step 1 checked"));
    assert!(checked.contains("Next step"));
    assert!(
        tool.handle_action(r#"{"action":"check_guards","command":"cargo test"}"#)
            .unwrap()
            .contains("satisfied")
    );
    assert!(
        tool.handle_action(r#"{"action":"skip","step":2}"#)
            .unwrap()
            .contains("skipped")
    );
    assert!(
        tool.handle_action(r#"{"action":"uncheck","step":"2"}"#)
            .unwrap()
            .contains("unchecked")
    );
    assert!(
        tool.handle_action(r#"{"action":"set_issue","issueNumber":7,"issueTitle":"bug"}"#)
            .unwrap()
            .contains("#7")
    );
    assert!(
        tool.handle_action(r#"{"action":"clear_issue"}"#)
            .unwrap()
            .contains("cleared")
    );
    assert!(
        tool.handle_action(r#"{"action":"reset"}"#)
            .unwrap()
            .contains("reset")
    );

    let emitted = events.lock().unwrap();
    assert!(emitted.iter().any(|e| e["type"] == "workflow_state"));
    assert!(emitted.iter().any(|e| e.get("activeIssue").is_some()));
}

#[tokio::test]
async fn execute_reports_parse_and_argument_errors_as_tool_results() {
    let tool = WorkflowTool::new(engine_handle());
    let bad_json = tool.execute("not json").await.unwrap();
    assert!(bad_json.is_error);
    assert!(bad_json.content.contains("invalid JSON"));

    let missing_action = tool.execute(r#"{}"#).await.unwrap();
    assert!(missing_action.is_error);
    assert!(missing_action.content.contains("missing required field"));

    let bad_step = tool
        .execute(r#"{"action":"check","step":{}}"#)
        .await
        .unwrap();
    assert!(bad_step.is_error);
    assert!(bad_step.content.contains("invalid step value"));
}

#[test]
fn workflow_guard_caches_rules_blocks_and_allows() {
    let handle = engine_handle();
    {
        let mut engine = handle.lock().unwrap();
        engine.select_template("feature", None).unwrap();
    }
    let guard = WorkflowGuard::new(handle.clone());
    let template = handle.lock().unwrap().active_template().unwrap().clone();
    let first = guard.parsed_rules_for(&template);
    let second = guard.parsed_rules_for(&template);
    assert!(Arc::ptr_eq(&first, &second));

    let blocked = guard
        .check("bash", r#"{"command":"cargo test --lib"}"#)
        .unwrap_err();
    assert!(blocked.contains("BLOCKED"));
    assert!(guard.check("read", r#"{}"#).is_ok());

    handle.lock().unwrap().check(1).unwrap();
    assert!(
        guard
            .check("bash", r#"{"command":"cargo test --lib"}"#)
            .is_ok()
    );
}

#[test]
fn workflow_guard_rejects_missing_template_and_invalid_config() {
    let no_template = WorkflowGuard::new(engine_handle());
    assert!(
        no_template
            .check("bash", r#"{"command":"cargo test"}"#)
            .unwrap_err()
            .contains("select a workflow template")
    );

    let config = WorkflowConfig {
        templates: vec![WorkflowTemplate {
            guards: vec![WorkflowGuardRule {
                commands: vec!["cargo test".into()],
                before_step_key: "missing".into(),
                message: "bad".into(),
            }],
            ..guarded_template("bad")
        }],
        ..WorkflowConfig::default()
    };
    let handle = Arc::new(Mutex::new(WorkflowEngine::new(config, true)));
    // Config validation rejects guards referencing unknown step keys up front.
    assert!(
        handle
            .lock()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("unknown step key")
    );
}

#[test]
fn select_template_rejects_an_unknown_template_id() {
    let tool = WorkflowTool::new(engine_handle());
    let err = tool
        .handle_action(r#"{"action":"select_template","template":"does-not-exist"}"#)
        .expect_err("selecting an unknown template must fail");
    assert!(!err.is_empty(), "error message should not be empty");
    // The failed selection must not leave a template bound.
    let status = tool
        .handle_action(r#"{"action":"status"}"#)
        .unwrap_or_default();
    assert!(
        !status.contains("does-not-exist"),
        "a rejected template must not become active: {status}"
    );
}

#[test]
fn guard_check_surfaces_a_poisoned_engine_rather_than_allowing_the_command() {
    let handle = engine_handle();
    handle
        .lock()
        .unwrap()
        .select_template("feature", None)
        .unwrap();
    let guard = WorkflowGuard::new(handle.clone());

    let poisoned = handle.clone();
    let _ = std::thread::spawn(move || {
        let _g = poisoned.lock().unwrap();
        panic!("poison the production workflow engine mutex");
    })
    .join();

    // Fail closed: a poisoned engine must not silently permit a guarded command.
    let err = guard
        .check("bash", r#"{"command":"cargo test"}"#)
        .expect_err("a poisoned engine must not allow the command through");
    assert!(
        err.contains("workflow engine poisoned"),
        "expected the poison message, got: {err}"
    );
}

#[test]
fn engine_construction_rejects_a_guard_rule_naming_a_missing_step() {
    // The guard's own `unknown step key` arm is unreachable defence-in-depth:
    // WorkflowEngine::new refuses such a config up front, so a rule referring to
    // a missing step can never reach ToolGuard::check. Pin that earlier gate.
    let mut template = guarded_template("broken");
    template.guards = vec![WorkflowGuardRule {
        commands: vec!["cargo test".into()],
        before_step_key: "no-such-step".into(),
        message: "unreachable".into(),
    }];
    let config = WorkflowConfig {
        templates: vec![template],
        ..WorkflowConfig::default()
    };

    let err = WorkflowEngine::new(config, true)
        .expect_err("a guard naming a missing step is an invalid configuration");
    let msg = err.to_string();
    assert!(
        msg.contains("no-such-step") && msg.contains("broken"),
        "error should name both the template and the bad step key: {msg}"
    );
}

#[test]
fn workflow_tool_debug_does_not_dump_engine_state() {
    // WorkflowTool holds the shared engine behind a mutex. Debug must stay a
    // bare struct name: deriving it would lock (or print) the whole template
    // set on every trace line, and could deadlock inside a held lock.
    let tool = WorkflowTool::new(engine_handle());
    let rendered = format!("{tool:?}");

    assert!(
        !rendered.contains("feature") && !rendered.contains("Mutex"),
        "Debug leaked engine internals: {rendered}"
    );
    assert!(
        rendered.len() < 64,
        "Debug should stay compact, got {} chars: {rendered}",
        rendered.len()
    );
}
