use super::app_events_test_support::test_app;
use super::*;

#[tokio::test]
async fn spawn_start_tracks_read_only_observer_args() {
    let mut app = test_app().await;
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-ro".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id":"reviewer","read_only":true}),
    });
    assert!(app.conn.roster.tracked["reviewer"].info.read_only);

    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-disabled".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id":"observer","disable_tools":["edit","write"]}),
    });
    assert!(app.conn.roster.tracked["observer"].info.read_only);
}

#[tokio::test]
async fn spawn_start_does_not_mark_partial_disable_as_observer() {
    let mut app = test_app().await;
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-partial".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id":"writer","disable_tools":["write"]}),
    });
    assert!(!app.conn.roster.tracked["writer"].info.read_only);
}
