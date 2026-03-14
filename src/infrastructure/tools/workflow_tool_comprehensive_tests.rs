//! Comprehensive additional tests for the WorkflowTool and WorkflowGuard.
//!
//! Covers string-typed number handling, full lifecycle, event emission edge
//! cases, guard behavior, error messages, and input validation.

use super::*;
use crate::domain::tool::{Tool, ToolGuard};
use crate::domain::workflow::{GuardRule, WorkflowState};
use std::sync::{Arc, Mutex};

fn test_tool() -> WorkflowTool {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    WorkflowTool::new(state)
}

fn test_tool_with_emitter() -> (WorkflowTool, Arc<Mutex<Vec<serde_json::Value>>>) {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    let events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(vec![]));
    let events_clone = events.clone();
    let emitter: WorkflowEventEmitter = Arc::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });
    (WorkflowTool::with_event_emitter(state, emitter), events)
}

// ─── String-typed number handling (Bug #6 fix) ──────────────────────────

#[tokio::test]
async fn test_check_with_string_step_number() {
    let tool = test_tool();
    let result = tool
        .execute(r#"{"action":"check","step":"1"}"#)
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "string step number should work: {}",
        result.content
    );
    assert!(result.content.contains("checked"));
}

#[tokio::test]
async fn test_skip_with_string_step_number() {
    let tool = test_tool();
    let result = tool
        .execute(r#"{"action":"skip","step":"5"}"#)
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "string step number should work: {}",
        result.content
    );
    assert!(result.content.contains("skipped"));
}

#[tokio::test]
async fn test_uncheck_with_string_step_number() {
    let tool = test_tool();
    tool.execute(r#"{"action":"check","step":1}"#)
        .await
        .unwrap();
    let result = tool
        .execute(r#"{"action":"uncheck","step":"1"}"#)
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "string step number should work: {}",
        result.content
    );
    assert!(result.content.contains("unchecked"));
}

#[tokio::test]
async fn test_set_issue_with_string_number() {
    let tool = test_tool();
    let result = tool
        .execute(r#"{"action":"set_issue","issueNumber":"42","issueTitle":"My feature"}"#)
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "string issue number should work: {}",
        result.content
    );
    assert!(result.content.contains("#42"));
}

#[tokio::test]
async fn test_check_with_invalid_string_step() {
    let tool = test_tool();
    let result = tool
        .execute(r#"{"action":"check","step":"abc"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid step value"));
}

#[tokio::test]
async fn test_set_issue_with_invalid_string_number() {
    let tool = test_tool();
    let result = tool
        .execute(r#"{"action":"set_issue","issueNumber":"abc","issueTitle":"test"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid issueNumber"));
}

#[tokio::test]
async fn test_check_with_step_exceeding_u32() {
    let tool = test_tool();
    let result = tool
        .execute(r#"{"action":"check","step":4294967297}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.content.contains("exceeds valid range"),
        "got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_check_with_float_step() {
    let tool = test_tool();
    let result = tool
        .execute(r#"{"action":"check","step":1.5}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid step value"));
}

#[tokio::test]
async fn test_check_with_null_step() {
    let tool = test_tool();
    let result = tool
        .execute(r#"{"action":"check","step":null}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    // null is neither u64 nor str, so parse_step returns "invalid step value"
    assert!(
        result.content.contains("invalid step value"),
        "got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_check_with_boolean_step() {
    let tool = test_tool();
    let result = tool
        .execute(r#"{"action":"check","step":true}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid step value"));
}

// ─── Full workflow lifecycle ────────────────────────────────────────────

#[tokio::test]
async fn test_full_workflow_lifecycle() {
    let (tool, events) = test_tool_with_emitter();

    // Set issue
    let r = tool
        .execute(r#"{"action":"set_issue","issueNumber":42,"issueTitle":"My feature"}"#)
        .await
        .unwrap();
    assert!(!r.is_error);

    // Check steps 1 through 6
    for i in 1..=6 {
        let r = tool
            .execute(&format!(r#"{{"action":"check","step":{}}}"#, i))
            .await
            .unwrap();
        assert!(!r.is_error, "step {} should check: {}", i, r.content);
    }

    // Status should show 6/16
    let r = tool.execute(r#"{"action":"status"}"#).await.unwrap();
    assert!(r.content.contains("6/16"));

    // Clear issue
    let r = tool.execute(r#"{"action":"clear_issue"}"#).await.unwrap();
    assert!(!r.is_error);

    // Reset
    let r = tool.execute(r#"{"action":"reset"}"#).await.unwrap();
    assert!(!r.is_error);

    // Status should show 0/16
    let r = tool.execute(r#"{"action":"status"}"#).await.unwrap();
    assert!(r.content.contains("0/16"));

    // Verify events emitted for each mutation (set_issue + 6 checks + clear_issue + reset = 9)
    let evts = events.lock().unwrap();
    assert_eq!(evts.len(), 9);
}

// ─── check_commit action in schema (Bug #4 fix) ────────────────────────

#[test]
fn test_definition_includes_check_commit_action() {
    let tool = test_tool();
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
    let action_strs: Vec<&str> = actions.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(
        action_strs.contains(&"check_commit"),
        "check_commit should be in action enum: {:?}",
        action_strs
    );
}

#[test]
fn test_definition_schema_is_valid_json() {
    let tool = test_tool();
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    assert!(schema["properties"]["action"]["enum"].is_array());
    assert!(schema["properties"]["step"]["type"].as_str() == Some("integer"));
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("action"))
    );
}

// ─── Event emission edge cases ──────────────────────────────────────────

#[tokio::test]
async fn test_event_not_emitted_on_invalid_json() {
    let (tool, events) = test_tool_with_emitter();
    tool.execute("not json").await.unwrap();
    let evts = events.lock().unwrap();
    assert_eq!(evts.len(), 0, "no event on invalid JSON");
}

#[tokio::test]
async fn test_event_not_emitted_on_unknown_action() {
    let (tool, events) = test_tool_with_emitter();
    tool.execute(r#"{"action":"bogus"}"#).await.unwrap();
    let evts = events.lock().unwrap();
    assert_eq!(evts.len(), 0, "no event on unknown action");
}

#[tokio::test]
async fn test_event_emitted_on_skip() {
    let (tool, events) = test_tool_with_emitter();
    tool.execute(r#"{"action":"skip","step":5}"#).await.unwrap();
    let evts = events.lock().unwrap();
    assert_eq!(evts.len(), 1);
    assert_eq!(evts[0]["type"], "workflow_state");
}

#[tokio::test]
async fn test_event_emitted_on_uncheck() {
    let (tool, events) = test_tool_with_emitter();
    tool.execute(r#"{"action":"check","step":1}"#)
        .await
        .unwrap();
    tool.execute(r#"{"action":"uncheck","step":1}"#)
        .await
        .unwrap();
    let evts = events.lock().unwrap();
    assert_eq!(evts.len(), 2);
}

#[tokio::test]
async fn test_event_emitted_on_clear_issue() {
    let (tool, events) = test_tool_with_emitter();
    tool.execute(r#"{"action":"set_issue","issueNumber":1,"issueTitle":"x"}"#)
        .await
        .unwrap();
    tool.execute(r#"{"action":"clear_issue"}"#).await.unwrap();
    let evts = events.lock().unwrap();
    assert_eq!(evts.len(), 2);
    // After clear_issue, activeIssue should not be in the event
    assert!(evts[1].get("activeIssue").is_none());
}

#[tokio::test]
async fn test_event_contains_correct_progress() {
    let (tool, events) = test_tool_with_emitter();
    tool.execute(r#"{"action":"check","step":1}"#)
        .await
        .unwrap();
    tool.execute(r#"{"action":"check","step":2}"#)
        .await
        .unwrap();
    let evts = events.lock().unwrap();
    assert_eq!(evts.len(), 2);
    assert_eq!(evts[1]["progress"]["done"], 2);
    assert_eq!(evts[1]["progress"]["total"], 16);
}

// ─── snapshot_to_event comprehensive ─────────────────────────────────────

#[test]
fn test_snapshot_to_event_step_fields() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    let event = snapshot_to_event(&state.snapshot());
    let step0 = &event["steps"][0];
    assert_eq!(step0["id"], 1);
    assert!(!step0["label"].as_str().unwrap().is_empty());
    assert!(!step0["phase"].as_str().unwrap().is_empty());
    assert_eq!(step0["done"], true);
}

#[test]
fn test_snapshot_to_event_progress_fields() {
    let state = WorkflowState::default_bdd();
    let event = snapshot_to_event(&state.snapshot());
    assert_eq!(event["progress"]["done"], 0);
    assert_eq!(event["progress"]["total"], 16);
    assert_eq!(event["progress"]["percent"], 0);
}

// ─── WorkflowGuard comprehensive ─────────────────────────────────────────

#[test]
fn test_guard_allows_non_bash_tools() {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    let guard = WorkflowGuard::new(
        state,
        vec![GuardRule {
            commands: vec!["git commit".into()],
            before_step: 7,
            message: "Not yet.".into(),
        }],
    );
    assert!(ToolGuard::check(&guard, "read", r#"{"path":"foo.rs"}"#).is_ok());
    assert!(ToolGuard::check(&guard, "write", r#"{"path":"foo.rs"}"#).is_ok());
    assert!(ToolGuard::check(&guard, "workflow", r#"{"action":"status"}"#).is_ok());
}

#[test]
fn test_guard_blocks_git_commit() {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    let guard = WorkflowGuard::new(
        state,
        vec![GuardRule {
            commands: vec!["git commit".into()],
            before_step: 7,
            message: "Finish RED-GREEN-REFACTOR first.".into(),
        }],
    );
    let result = ToolGuard::check(&guard, "bash", r#"{"command":"git commit -m wip"}"#);
    assert!(result.is_err());
    let err_msg = result.unwrap_err();
    assert!(
        err_msg.contains("BLOCKED"),
        "should contain BLOCKED: {}",
        err_msg
    );
}

#[test]
fn test_guard_allows_git_commit_after_steps() {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    {
        let mut s = state.lock().unwrap();
        for i in 1..=6 {
            s.check(i).unwrap();
        }
    }
    let guard = WorkflowGuard::new(
        state,
        vec![GuardRule {
            commands: vec!["git commit".into()],
            before_step: 7,
            message: "Finish first.".into(),
        }],
    );
    assert!(ToolGuard::check(&guard, "bash", r#"{"command":"git commit -m done"}"#).is_ok());
}

#[test]
fn test_guard_no_rules_allows_everything() {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    let guard = WorkflowGuard::new(state, vec![]);
    assert!(ToolGuard::check(&guard, "bash", r#"{"command":"git commit -m yolo"}"#).is_ok());
}

#[test]
fn test_guard_multiple_rules() {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    let guard = WorkflowGuard::new(
        state,
        vec![
            GuardRule {
                commands: vec!["git commit".into(), "git push".into()],
                before_step: 7,
                message: "Complete first 6 steps.".into(),
            },
            GuardRule {
                commands: vec!["gh pr merge".into()],
                before_step: 15,
                message: "Complete review.".into(),
            },
        ],
    );
    assert!(ToolGuard::check(&guard, "bash", r#"{"command":"git commit -m x"}"#).is_err());
    assert!(ToolGuard::check(&guard, "bash", r#"{"command":"gh pr merge 42"}"#).is_err());
    assert!(ToolGuard::check(&guard, "bash", r#"{"command":"git status"}"#).is_ok());
}

#[test]
fn test_guard_block_message_includes_blocked_prefix() {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    let guard = WorkflowGuard::new(
        state,
        vec![GuardRule {
            commands: vec!["git commit".into()],
            before_step: 7,
            message: "Custom message here.".into(),
        }],
    );
    let result = ToolGuard::check(&guard, "bash", r#"{"command":"git commit -m x"}"#);
    let err = result.unwrap_err();
    assert!(
        err.starts_with("BLOCKED:"),
        "should start with BLOCKED: got: {}",
        err
    );
    assert!(err.contains("Custom message here."));
    assert!(err.contains("workflow"));
}

#[test]
fn test_guard_with_empty_command() {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    let guard = WorkflowGuard::new(
        state,
        vec![GuardRule {
            commands: vec!["git commit".into()],
            before_step: 7,
            message: "Not yet.".into(),
        }],
    );
    assert!(ToolGuard::check(&guard, "bash", r#"{"command":""}"#).is_ok());
}

#[test]
fn test_guard_with_malformed_json() {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    let guard = WorkflowGuard::new(
        state,
        vec![GuardRule {
            commands: vec!["git commit".into()],
            before_step: 7,
            message: "Not yet.".into(),
        }],
    );
    assert!(ToolGuard::check(&guard, "bash", "not json").is_ok());
}

// ─── Tool state sharing ─────────────────────────────────────────────────

#[tokio::test]
async fn test_tool_state_accessor() {
    let tool = test_tool();
    let state = tool.state();
    let s = state.lock().unwrap();
    assert_eq!(s.steps().len(), 16);
}

// ─── Set and mutate guards ──────────────────────────────────────────────

#[tokio::test]
async fn test_set_guards_after_creation() {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    let mut tool = WorkflowTool::new(state);
    // Initially no guards
    let r = tool.execute(r#"{"action":"check_commit"}"#).await.unwrap();
    assert!(!r.is_error);
    // Add guards
    tool.set_guards(vec![GuardRule {
        commands: vec!["git commit".into()],
        before_step: 7,
        message: "Blocked.".into(),
    }]);
    let r = tool.execute(r#"{"action":"check_commit"}"#).await.unwrap();
    assert!(r.is_error);
}

// ─── Set event emitter after creation ───────────────────────────────────

#[tokio::test]
async fn test_set_event_emitter_after_creation() {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    let mut tool = WorkflowTool::new(state);
    let events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(vec![]));
    let events_clone = events.clone();
    tool.set_event_emitter(Arc::new(move |event| {
        events_clone.lock().unwrap().push(event);
    }));
    tool.execute(r#"{"action":"check","step":1}"#)
        .await
        .unwrap();
    let evts = events.lock().unwrap();
    assert_eq!(evts.len(), 1);
}

// ─── Error messages are user-friendly ───────────────────────────────────

#[tokio::test]
async fn test_ordering_error_mentions_specific_step() {
    let tool = test_tool();
    tool.execute(r#"{"action":"check","step":1}"#)
        .await
        .unwrap();
    tool.execute(r#"{"action":"check","step":2}"#)
        .await
        .unwrap();
    let r = tool
        .execute(r#"{"action":"check","step":4}"#)
        .await
        .unwrap();
    assert!(r.is_error);
    assert!(
        r.content.contains("complete step 3 first"),
        "should mention the missing step: {}",
        r.content
    );
}

#[tokio::test]
async fn test_out_of_range_error_message() {
    let tool = test_tool();
    let r = tool
        .execute(r#"{"action":"check","step":99}"#)
        .await
        .unwrap();
    assert!(r.is_error);
    assert!(r.content.contains("invalid step 99"));
    assert!(r.content.contains("must be 1-16"));
}

// ─── Action is case-sensitive ───────────────────────────────────────────

#[tokio::test]
async fn test_action_case_sensitive() {
    let tool = test_tool();
    let r = tool.execute(r#"{"action":"Status"}"#).await.unwrap();
    assert!(r.is_error);
    assert!(r.content.contains("unknown action"));
}

#[tokio::test]
async fn test_action_check_uppercase() {
    let tool = test_tool();
    let r = tool
        .execute(r#"{"action":"CHECK","step":1}"#)
        .await
        .unwrap();
    assert!(r.is_error);
    assert!(r.content.contains("unknown action"));
}

// ─── Missing step field for check/uncheck/skip ──────────────────────────

#[tokio::test]
async fn test_check_without_step_field() {
    let tool = test_tool();
    let r = tool.execute(r#"{"action":"check"}"#).await.unwrap();
    assert!(r.is_error);
    assert!(r.content.contains("missing field: step"));
}

#[tokio::test]
async fn test_uncheck_without_step_field() {
    let tool = test_tool();
    let r = tool.execute(r#"{"action":"uncheck"}"#).await.unwrap();
    assert!(r.is_error);
    assert!(r.content.contains("missing field: step"));
}

#[tokio::test]
async fn test_skip_without_step_field() {
    let tool = test_tool();
    let r = tool.execute(r#"{"action":"skip"}"#).await.unwrap();
    assert!(r.is_error);
    assert!(r.content.contains("missing field: step"));
}

// ─── Negative step values ───────────────────────────────────────────────

#[tokio::test]
async fn test_check_with_negative_step() {
    let tool = test_tool();
    let r = tool
        .execute(r#"{"action":"check","step":-1}"#)
        .await
        .unwrap();
    assert!(r.is_error);
    assert!(
        r.content.contains("invalid step value") || r.content.contains("missing field"),
        "got: {}",
        r.content
    );
}

// ─── set_issue edge cases ───────────────────────────────────────────────

#[tokio::test]
async fn test_set_issue_with_empty_title() {
    let tool = test_tool();
    let r = tool
        .execute(r#"{"action":"set_issue","issueNumber":1,"issueTitle":""}"#)
        .await
        .unwrap();
    assert!(!r.is_error);
    assert!(r.content.contains("#1"));
}

#[tokio::test]
async fn test_set_issue_with_special_chars_in_title() {
    let tool = test_tool();
    let r = tool
        .execute(r#"{"action":"set_issue","issueNumber":1,"issueTitle":"Fix \"quotes\" & <tags>"}"#)
        .await
        .unwrap();
    assert!(!r.is_error);
}

#[tokio::test]
async fn test_set_issue_number_zero() {
    let tool = test_tool();
    let r = tool
        .execute(r#"{"action":"set_issue","issueNumber":0,"issueTitle":"Zero"}"#)
        .await
        .unwrap();
    assert!(!r.is_error);
    assert!(r.content.contains("#0"));
}

#[tokio::test]
async fn test_set_issue_large_number() {
    let tool = test_tool();
    let r = tool
        .execute(r#"{"action":"set_issue","issueNumber":4294967295,"issueTitle":"Max u32"}"#)
        .await
        .unwrap();
    assert!(!r.is_error);
}

#[tokio::test]
async fn test_set_issue_number_exceeds_u32() {
    let tool = test_tool();
    let r = tool
        .execute(r#"{"action":"set_issue","issueNumber":4294967296,"issueTitle":"Over u32"}"#)
        .await
        .unwrap();
    assert!(r.is_error);
    assert!(r.content.contains("exceeds u32 range"));
}

// ─── Status output format ───────────────────────────────────────────────

#[tokio::test]
async fn test_status_shows_all_step_labels() {
    let tool = test_tool();
    let r = tool.execute(r#"{"action":"status"}"#).await.unwrap();
    assert!(r.content.contains("Update Scenarios"));
    assert!(r.content.contains("Write/update unit tests"));
    assert!(r.content.contains("Move to local master and pull"));
}

#[tokio::test]
async fn test_status_shows_current_step() {
    let tool = test_tool();
    let r = tool.execute(r#"{"action":"status"}"#).await.unwrap();
    assert!(r.content.contains("CURRENT STEP → 1."));
}

#[tokio::test]
async fn test_status_after_check_shows_next_current() {
    let tool = test_tool();
    tool.execute(r#"{"action":"check","step":1}"#)
        .await
        .unwrap();
    let r = tool.execute(r#"{"action":"status"}"#).await.unwrap();
    assert!(r.content.contains("CURRENT STEP → 2."));
    assert!(r.content.contains("[✓] 1."));
}
