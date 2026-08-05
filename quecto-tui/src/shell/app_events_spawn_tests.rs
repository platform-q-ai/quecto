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
        runtime_backend: "local".to_string(),
        container_uuid: None,
        container_ref: None,
        container_name: None,
        repo_url: None,
        environment_id: None,
        workspace_path: None,
        environment_health: None,
        socket_mode: None,
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

/// #1378 adversarial re-review: ToolEnd uuid= rekey must migrate sessions /
/// feeds / session_order with tracked+active, or focus under UUID orphans the
/// pre-rekey transcript under the display key.
#[tokio::test]
async fn tool_end_uuid_rekey_migrates_sessions_feeds_and_session_order() {
    let mut app = test_app().await;
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id": "worker-1"}),
    });
    // User focuses the optimistic row before uuid arrives → session under display.
    app.select_agent(Some("worker-1"));
    assert!(app.subagents.sessions.contains_key("worker-1"));
    assert_eq!(app.subagents.active_agent_id.as_deref(), Some("worker-1"));
    // Seed a distinguishable transcript so we can prove migration, not recreate.
    app.subagents
        .sessions
        .get_mut("worker-1")
        .unwrap()
        .chat
        .append_token("pre-rekey transcript");
    // Synthetic feed under the display key (as ensure_session/select would pair).
    let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel(1);
    app.subagents.feeds.insert(
        "worker-1".into(),
        crate::agents::view::FeedState {
            cmd_tx,
            handle: tokio::spawn(async {}),
            epoch: 0,
            rev: 0,
            last_fresh_at: None,
            supports_sync: false,
            pending_rev: None,
            transcript: crate::agents::ledger::LedgerTranscript::default(),
            authority: crate::agents::feed::FeedAuthority::SyncedAuthoritative,
        },
    );
    assert!(
        app.subagents
            .session_order
            .iter()
            .any(|id| id == "worker-1")
    );

    let uuid = "66666666-6666-4666-8666-666666666666";
    app.handle_event(Event::ToolExecutionEnd {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        result: serde_json::json!({
            "content": [{
                "type": "text",
                "text": format!("Subagent 'worker-1' is running (uuid={uuid})")
            }]
        }),
        is_error: false,
    });

    assert!(
        !app.subagents.tracked.contains_key("worker-1"),
        "tracked must leave the display key"
    );
    assert!(app.subagents.tracked.contains_key(uuid));
    assert_eq!(app.subagents.active_agent_id.as_deref(), Some(uuid));
    assert!(
        !app.subagents.sessions.contains_key("worker-1"),
        "sessions must rekey with tracked"
    );
    assert!(
        app.subagents.sessions.contains_key(uuid),
        "session must land under UUID"
    );
    let migrated = app.subagents.sessions[uuid]
        .chat
        .entries()
        .iter()
        .any(|e| match e {
            crate::components::chat::ChatEntry::Assistant { text, .. } => {
                text.contains("pre-rekey transcript")
            }
            _ => false,
        });
    assert!(
        migrated,
        "pre-rekey transcript must migrate, not be orphaned"
    );
    assert!(
        !app.subagents.feeds.contains_key("worker-1"),
        "feeds must rekey with tracked"
    );
    assert!(app.subagents.feeds.contains_key(uuid));
    assert!(
        !app.subagents
            .session_order
            .iter()
            .any(|id| id == "worker-1"),
        "session_order must drop display key"
    );
    assert!(
        app.subagents.session_order.iter().any(|id| id == uuid),
        "session_order must carry UUID key"
    );
}

/// #1378: snapshot optimistic migrate must also move sessions/feeds/order.
#[tokio::test]
async fn snapshot_uuid_migrate_moves_sessions_feeds_and_session_order() {
    let mut app = test_app().await;
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id": "worker-1"}),
    });
    app.select_agent(Some("worker-1"));
    app.subagents
        .sessions
        .get_mut("worker-1")
        .unwrap()
        .chat
        .append_token("snapshot-migrate");
    let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel(1);
    app.subagents.feeds.insert(
        "worker-1".into(),
        crate::agents::view::FeedState {
            cmd_tx,
            handle: tokio::spawn(async {}),
            epoch: 0,
            rev: 0,
            last_fresh_at: None,
            supports_sync: false,
            pending_rev: None,
            transcript: crate::agents::ledger::LedgerTranscript::default(),
            authority: crate::agents::feed::FeedAuthority::SyncedAuthoritative,
        },
    );

    let uuid = "77777777-7777-4777-8777-777777777777";
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
        runtime_backend: "local".to_string(),
        container_uuid: None,
        container_ref: None,
        container_name: None,
        repo_url: None,
        environment_id: None,
        workspace_path: None,
        environment_health: None,
        socket_mode: None,
    }]);

    assert!(!app.subagents.sessions.contains_key("worker-1"));
    assert!(app.subagents.sessions.contains_key(uuid));
    let migrated = app.subagents.sessions[uuid]
        .chat
        .entries()
        .iter()
        .any(|e| match e {
            crate::components::chat::ChatEntry::Assistant { text, .. } => {
                text.contains("snapshot-migrate")
            }
            _ => false,
        });
    assert!(migrated, "snapshot migrate must keep session content");
    assert!(!app.subagents.feeds.contains_key("worker-1"));
    assert!(app.subagents.feeds.contains_key(uuid));
    assert!(
        !app.subagents
            .session_order
            .iter()
            .any(|id| id == "worker-1")
    );
    assert!(app.subagents.session_order.iter().any(|id| id == uuid));
    assert_eq!(app.subagents.active_agent_id.as_deref(), Some(uuid));
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
