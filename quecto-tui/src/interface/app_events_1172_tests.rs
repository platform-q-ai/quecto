use super::app_events_test_support::test_app;
use super::*;

#[tokio::test]
async fn background_forwarded_child_workflow_state_updates_left_panel_snapshot() {
    let mut app = test_app().await;
    app.handle_event(Event::SubagentStateChanged {
        subagents: vec![crate::infrastructure::client::SubagentInfoEvent {
            agent_id: "child".into(),
            status: "running".into(),
            last_tool: None,
            last_error: None,
            pid: 0,
            socket_path: None,
            parent_id: None,
            workflow: None,
            read_only: false,
        }],
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
