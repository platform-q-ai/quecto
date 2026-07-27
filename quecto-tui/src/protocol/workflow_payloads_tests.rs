use super::*;
use serde_json::json;

#[test]
fn parse_workflow_snapshot_reads_v2_steps_and_progress() {
    let snap = parse_workflow_snapshot(&json!({
        "steps": [{"index": 0, "label": "Build", "phase": "build", "done": false}],
        "progress": {"done": 0, "total": 1},
        "activeIssue": {"number": 7, "title": "thing"},
        "mode": "active",
        "automation": {"autoContinue": true, "completionNudge": false},
    }));
    assert_eq!(snap.steps.len(), 1);
    assert_eq!(snap.steps[0].id, 0);
    assert_eq!(snap.done, 0);
    assert_eq!(snap.total, 1);
    assert_eq!(snap.issue_number, Some(7));
    assert_eq!(snap.issue_title.as_deref(), Some("thing"));
    assert!(snap.workflow_auto_continue);
    assert!(!snap.workflow_completion_nudge);
}

#[test]
fn parse_workflow_snapshot_accepts_snake_case_and_array_issue() {
    let snap = parse_workflow_snapshot(&json!({
        "steps": [{"id": 2, "label": "X", "phase": "red", "done": true}],
        "active_issue": [9, "title"],
        "active_template": {"label": "TDD"},
        "available_templates": [{}, {}],
        "automation": {"auto_continue": true, "completion_nudge": true},
    }));
    assert_eq!(snap.steps[0].id, 2);
    assert_eq!(snap.issue_number, Some(9));
    assert_eq!(snap.issue_title.as_deref(), Some("title"));
    assert_eq!(snap.template_name.as_deref(), Some("TDD"));
    assert_eq!(snap.template_count, 2);
    assert!(snap.workflow_auto_continue);
    assert!(snap.workflow_completion_nudge);
}

#[test]
fn parse_workflow_automation_top_level_or_nested() {
    let nested = parse_workflow_automation(&json!({
        "automation": {"autoContinue": true}
    }));
    assert_eq!(nested.auto_continue, Some(true));
    assert_eq!(nested.completion_nudge, None);

    let top = parse_workflow_automation(&json!({"autoContinue": false, "completionNudge": true}));
    assert_eq!(top.auto_continue, Some(false));
    assert_eq!(top.completion_nudge, Some(true));
}

#[test]
fn parse_workflow_snapshot_defaults_missing_automation_to_false() {
    let snap = parse_workflow_snapshot(&json!({
        "steps": [{"index": 0, "label": "A", "phase": "build", "done": false}]
    }));
    assert!(!snap.workflow_auto_continue);
    assert!(!snap.workflow_completion_nudge);
}
