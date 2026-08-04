use super::app_events_test_support::test_app;
use super::*;

#[tokio::test]
async fn track_starting_subagent_without_agent_id_is_noop() {
    let mut app = test_app().await;
    app.spinner = Some(Spinner::new("Working"));
    // spawn tool with no agent_id → track_starting_subagent should bail.
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"task": "do something"}),
    });
    // No subagent should be tracked.
    assert!(
        app.subagents.tracked.is_empty(),
        "spawn without agent_id should not track a subagent"
    );
}

#[tokio::test]
async fn track_starting_subagent_strips_control_chars_from_id() {
    let mut app = test_app().await;
    app.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id": "a\u{0007}b"}),
    });
    // The sanitized id should be stored, not the raw one.
    assert!(
        app.subagents.tracked.contains_key("ab"),
        "control chars should be stripped from agent_id"
    );
    assert!(
        !app.subagents.tracked.contains_key("a\u{0007}b"),
        "raw (unsanitized) id should not be a key"
    );
}

#[tokio::test]
async fn mark_spawned_subagent_running_with_no_quotes_is_noop() {
    let mut app = test_app().await;
    app.spinner = Some(Spinner::new("Working"));
    // First, track a subagent via spawn start.
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id": "worker-1"}),
    });
    assert!(app.subagents.tracked.contains_key("worker-1"));
    assert_eq!(app.subagents.tracked["worker-1"].info.status, "starting");

    // Tool end with result text that has NO single quotes.
    app.handle_event(Event::ToolExecutionEnd {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        result: serde_json::json!({
            "content": [{"type": "text", "text": "Subagent started successfully"}]
        }),
        is_error: false,
    });
    // Status should remain "starting" (not updated to "running").
    assert_eq!(
        app.subagents.tracked["worker-1"].info.status, "starting",
        "malformed result (no quotes) should not update status"
    );
}

#[tokio::test]
async fn mark_spawned_subagent_running_with_one_quote_is_noop() {
    let mut app = test_app().await;
    app.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id": "worker-1"}),
    });

    // Only one quote — can't find the closing quote.
    app.handle_event(Event::ToolExecutionEnd {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        result: serde_json::json!({
            "content": [{"type": "text", "text": "Subagent 'worker-1 started"}]
        }),
        is_error: false,
    });
    assert_eq!(
        app.subagents.tracked["worker-1"].info.status, "starting",
        "result with only one quote should not update status"
    );
}

#[tokio::test]
async fn handle_tool_end_spawn_error_does_not_mark_running() {
    let mut app = test_app().await;
    app.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id": "worker-1"}),
    });

    // Tool end with is_error=true → should NOT call mark_spawned_subagent_running.
    app.handle_event(Event::ToolExecutionEnd {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        result: serde_json::json!({
            "content": [{"type": "text", "text": "Subagent 'worker-1' is running"}]
        }),
        is_error: true,
    });
    assert_eq!(
        app.subagents.tracked["worker-1"].info.status, "starting",
        "error result should not mark subagent as running"
    );
}

/// #1378: snapshot arriving under UUID must absorb a display-keyed optimistic
/// row instead of leaving dual entries for the grace window.
#[tokio::test]
async fn optimistic_display_row_reconciles_to_uuid_snapshot_without_dual_rows() {
    let mut app = test_app().await;
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id": "worker-1"}),
    });
    assert!(app.subagents.tracked.contains_key("worker-1"));
    assert!(app.subagents.tracked["worker-1"].optimistic);

    let uuid = "55555555-5555-4555-8555-555555555555";
    app.update_subagent_bar(vec![crate::protocol::client::SubagentInfoEvent {
        agent_uuid: Some(uuid.into()),
        display_name: Some("worker-1".into()),
        agent_id: "worker-1".into(),
        status: "running".into(),
        last_tool: None,
        last_error: None,
        pid: 1,
        socket_path: None,
        parent_id: None,
        workflow: None,
        read_only: false,
    }]);

    assert!(
        !app.subagents.tracked.contains_key("worker-1"),
        "display-keyed optimistic row must not survive next to UUID row"
    );
    assert!(
        app.subagents.tracked.contains_key(uuid),
        "authoritative UUID row must be present"
    );
    assert!(!app.subagents.tracked[uuid].optimistic);
    assert_eq!(app.subagents.tracked[uuid].info.status, "running");
    assert_eq!(
        app.subagents.tracked[uuid].info.display_name.as_deref(),
        Some("worker-1")
    );
}

#[tokio::test]
async fn mark_spawned_subagent_running_with_unknown_id_is_noop() {
    let mut app = test_app().await;
    app.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id": "worker-1"}),
    });

    // Result text mentions a DIFFERENT agent_id that's not tracked.
    app.handle_event(Event::ToolExecutionEnd {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        result: serde_json::json!({
            "content": [{"type": "text", "text": "Subagent 'unknown-agent' is running"}]
        }),
        is_error: false,
    });
    // worker-1 should remain "starting"; unknown-agent was never tracked.
    assert_eq!(app.subagents.tracked["worker-1"].info.status, "starting");
    assert!(!app.subagents.tracked.contains_key("unknown-agent"));
}
