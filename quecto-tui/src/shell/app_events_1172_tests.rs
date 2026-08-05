use super::app_events_test_support::test_app;
use super::*;
use crate::protocol::client::SubagentInfoEvent;

fn tracked_child() -> SubagentInfoEvent {
    SubagentInfoEvent {
        agent_uuid: None,
        display_name: None,
        agent_id: "child".into(),
        status: "running".into(),
        last_tool: None,
        last_error: None,
        pid: 0,
        socket_path: None,
        parent_id: None,
        workflow: None,
        read_only: false,
        runtime_backend: "local".to_string(),
        container_uuid: None,
        container_ref: None,
        container_name: None,
        repo_url: None,
        environment_id: None,
        workspace_path: None,
        environment_health: None,
        socket_mode: None,
    }
}

#[tokio::test]
async fn background_forwarded_child_workflow_state_updates_left_panel_snapshot() {
    let mut app = test_app().await;
    app.handle_event(Event::SubagentStateChanged {
        subagents: vec![tracked_child()],
    });

    app.handle_event(Event::WorkflowState {
        agent_id: Some("child".into()),
        steps: vec![],
        progress: serde_json::json!({"done": 4, "total": 7}),
        active_issue: None,
        mode: Some("active".into()),
        active_template: None,
        available_templates: None,
    });

    let wf = app
        .subagents
        .tracked
        .get("child")
        .and_then(|entry| entry.info.workflow.as_ref())
        .expect("background forwarded workflow_state should update the left-panel model");
    assert_eq!(wf.steps_completed, 4);
    assert_eq!(wf.steps_total, 7);
    assert_eq!(
        app.master_session.workflow_bar.total, 0,
        "child progress must not clobber master bar"
    );
}

#[tokio::test]
async fn background_forwarded_empty_workflow_does_not_regress_visible_child_progress() {
    let mut app = test_app().await;
    app.handle_event(Event::SubagentStateChanged {
        subagents: vec![tracked_child()],
    });

    app.handle_event(Event::WorkflowState {
        agent_id: Some("child".into()),
        steps: vec![],
        progress: serde_json::json!({"done": 3, "total": 5}),
        active_issue: None,
        mode: Some("active".into()),
        active_template: None,
        available_templates: None,
    });
    app.handle_event(Event::WorkflowState {
        agent_id: Some("child".into()),
        steps: vec![],
        progress: serde_json::json!({"done": 0, "total": 0}),
        active_issue: None,
        mode: Some("active".into()),
        active_template: None,
        available_templates: None,
    });

    let wf = app
        .subagents
        .tracked
        .get("child")
        .and_then(|entry| entry.info.workflow.as_ref())
        .expect("child workflow should remain tracked");
    assert_eq!(wf.steps_completed, 3);
    assert_eq!(wf.steps_total, 5);
}

#[tokio::test]
async fn background_forwarded_completed_workflow_records_completed_mode() {
    let mut app = test_app().await;
    app.handle_event(Event::SubagentStateChanged {
        subagents: vec![tracked_child()],
    });

    app.handle_event(Event::WorkflowState {
        agent_id: Some("child".into()),
        steps: vec![],
        progress: serde_json::json!({"done": 2, "total": 3}),
        active_issue: None,
        mode: Some("active".into()),
        active_template: None,
        available_templates: None,
    });
    app.handle_event(Event::WorkflowState {
        agent_id: Some("child".into()),
        steps: vec![],
        progress: serde_json::json!({"done": 3, "total": 3}),
        active_issue: None,
        mode: Some("complete".into()),
        active_template: None,
        available_templates: None,
    });

    let wf = app
        .subagents
        .tracked
        .get("child")
        .and_then(|entry| entry.info.workflow.as_ref())
        .expect("child workflow should remain tracked");
    assert_eq!(wf.mode, "complete");
    assert_eq!(wf.steps_completed, 3);
    assert_eq!(wf.steps_total, 3);
}
