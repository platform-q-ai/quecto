use super::*;
use crate::protocol::client::ChildToolPolicyPropagation;

#[test]
fn tool_policy_child_propagation_failures_are_visible() {
    let mut app = App::new(Terminal::new(), Client::disconnected_for_tests());

    app.handle_event(Event::ToolPolicyChanged {
        changed_tools: vec!["bash".into()],
        results: vec![],
        child_propagation: vec![
            ChildToolPolicyPropagation {
                agent_id: "child-ok".into(),
                status: "queued".into(),
                extra: Default::default(),
            },
            ChildToolPolicyPropagation {
                agent_id: "child-bad".into(),
                status: "timeout".into(),
                extra: Default::default(),
            },
        ],
        apply_mode: "atNextTurnBoundary".into(),
        reason: "policy".into(),
    });

    let rendered = app
        .master_session
        .chat
        .last_status_text()
        .expect("expected visible policy status note");
    assert!(
        rendered.contains("1 child policy update failed") && rendered.contains("1 queued/applied"),
        "notification must summarize failed child propagation accurately: {rendered}"
    );
}
