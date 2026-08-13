use super::app_events_test_support::test_app;
use super::*;

#[tokio::test]
async fn subagent_notification_appends_one_status_line() {
    let mut app = test_app().await;
    let before = app.conn.master_session.chat.entry_count();
    app.handle_event(Event::SubagentNotification {
        agent_id: "researcher".into(),
        sequence: 1,
        message: "Agent 'researcher' completed and is ready for inspection".into(),
    });
    // Exactly one status entry is appended — passive, non-interactive.
    assert_eq!(app.conn.master_session.chat.entry_count(), before + 1);
    let text = app
        .conn
        .master_session
        .chat
        .last_status_text()
        .expect("expected a Status entry");
    assert!(text.contains("Agent 'researcher' completed"));
    assert!(text.contains("ready for inspection"));
    // The TUI must NOT re-prefix the agent id — the message already names it.
    assert!(!text.contains("sub-agent researcher"));
}

#[tokio::test]
async fn subagent_notification_sanitizes_control_sequences() {
    let mut app = test_app().await;
    app.handle_event(Event::SubagentNotification {
        agent_id: "evil".into(),
        sequence: 1,
        message: "done\u{1b}[31m hijack".into(),
    });
    let text = app
        .conn
        .master_session
        .chat
        .last_status_text()
        .expect("expected a Status entry");
    assert!(
        !text.contains('\u{1b}'),
        "control sequences must be stripped"
    );
}

#[tokio::test]
async fn subagent_notification_deferred_while_parent_streams_then_flushed_on_idle() {
    let mut app = test_app().await;
    app.handle_event(Event::AgentStart);
    let before = app.conn.master_session.chat.entry_count();
    app.handle_event(Event::SubagentNotification {
        agent_id: "worker".into(),
        sequence: 1,
        message: "Agent 'worker' completed and is ready for inspection".into(),
    });
    // Mid-turn: the note must NOT be inserted into the streaming response.
    assert_eq!(
        app.conn.master_session.chat.entry_count(),
        before,
        "note must be deferred while the parent is streaming"
    );
    // Parent goes idle → the note is flushed after the finished response.
    app.handle_event(Event::AgentEnd {
        messages: vec![],
        message_refs: vec![],
    });
    let text = app
        .conn
        .master_session
        .chat
        .last_status_text()
        .expect("expected the deferred note to flush on idle");
    assert!(text.contains("Agent 'worker' completed"));
}

#[tokio::test]
async fn handles_agent_lifecycle_and_token_events() {
    let mut app = test_app().await;
    app.handle_event(Event::AgentStart);
    assert!(app.conn.agent_state.is_running());
    assert!(app.conn.spinner.is_some());
    app.handle_event(Event::Token {
        token: "hello".into(),
    });
    app.handle_event(Event::TurnStart);
    app.handle_event(Event::AgentEnd {
        messages: vec![],
        message_refs: vec![],
    });
    assert!(!app.conn.agent_state.is_running());
    assert!(app.conn.spinner.is_none());
}

#[tokio::test]
async fn handles_turn_end_usage_with_context_window_and_stats_fallback() {
    let mut app = test_app().await;
    app.handle_event(Event::TurnEnd {
        message: serde_json::json!({
            "usage": {"input": 10, "output": 5, "total": 15},
            "contextTokens": 40,
            "maxContextTokens": 100
        }),
    });
    assert!(app.conn.sessions.context_stats_requested);
    let rendered = app.conn.master_session.footer.render(80).join("\n");
    assert!(
        rendered.contains("40/100"),
        "footer should use contextTokens: {rendered}"
    );

    let mut app = test_app().await;
    app.handle_event(Event::TurnEnd {
        message: serde_json::json!({"usage": {"total": 1}}),
    });
    assert!(app.conn.sessions.context_stats_requested);
}

#[tokio::test]
async fn handles_turn_end_context_tokens_without_usage_field() {
    // Streaming OpenAI-compatible providers (e.g. Fireworks) emit
    // `contextTokens`/`maxContextTokens` but no `usage`. The footer must
    // still update rather than bailing out early on the missing `usage`.
    let mut app = test_app().await;
    app.handle_event(Event::TurnEnd {
        message: serde_json::json!({
            "contextTokens": 40,
            "maxContextTokens": 100
        }),
    });
    assert!(app.conn.sessions.context_stats_requested);
    let rendered = app.conn.master_session.footer.render(80).join("\n");
    assert!(
        rendered.contains("40/100"),
        "footer should use contextTokens even without usage: {rendered}"
    );
}

#[tokio::test]
async fn session_stats_footer_uses_context_tokens_not_cumulative_input() {
    let mut app = test_app().await;
    app.handle_event(Event::Response {
        id: None,
        command: "get_session_stats".into(),
        success: true,
        data: Some(serde_json::json!({
            "tokens": {"input": 999_000, "output": 1, "total": 999_001},
            "contextTokens": 12_000,
            "maxContextTokens": 200_000
        })),
        error: None,
    });

    let rendered = app.conn.master_session.footer.render(80).join("\n");
    let plain: String = rendered
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    assert!(
        plain.contains("12k/200k"),
        "footer should show active context: {plain}"
    );
    assert!(
        !plain.contains("999k/200k"),
        "footer must not show cumulative input: {plain}"
    );
}

#[tokio::test]
async fn handles_tool_start_and_end_for_spawn_and_regular_tools() {
    let mut app = test_app().await;
    app.conn.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id":"worker-1"}),
    });
    assert!(app.conn.roster.tracked.contains_key("worker-1"));

    app.handle_event(Event::ToolExecutionEnd {
            tool_call_id: "spawn-1".into(),
            tool_name: "spawn".into(),
            result: serde_json::json!({"content":[{"type":"text","text":"Subagent 'worker-1' is running (uuid=33333333-3333-4333-8333-333333333333)"}]}),
            is_error: false,
        });
    // #1378: ToolEnd rekeys the optimistic display row onto the durable UUID.
    assert!(
        !app.conn.roster.tracked.contains_key("worker-1"),
        "display-keyed optimistic row must be migrated"
    );
    let uuid = "33333333-3333-4333-8333-333333333333";
    assert_eq!(app.conn.roster.tracked[uuid].info.status, "running");
    assert_eq!(
        app.conn.roster.tracked[uuid].info.display_name.as_deref(),
        Some("worker-1")
    );

    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "read-1".into(),
        tool_name: "read".into(),
        args: serde_json::json!({"path":"file.txt"}),
    });
    app.handle_event(Event::ToolExecutionEnd {
        tool_call_id: "read-1".into(),
        tool_name: "read".into(),
        result: serde_json::json!({"content":[{"type":"text","text":"contents"}]}),
        is_error: false,
    });
}

/// A read-only `agent_cmd` query (#865) renders a tool box on the master path.
#[tokio::test]
async fn agent_cmd_get_state_renders_box_on_master_path() {
    let mut app = test_app().await;
    let before = app.conn.master_session.chat.entry_count();
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "gs-1".into(),
        tool_name: "agent_cmd".into(),
        args: serde_json::json!({"agent_id":"worker-1", "command":"get_state"}),
    });
    let after = app.conn.master_session.chat.entry_count();
    assert_eq!(after, before + 1, "get_state renders a box");
}

#[tokio::test]
async fn handles_agent_cmd_spinner_and_subagent_refresh() {
    let mut app = test_app().await;
    app.conn.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "cmd-1".into(),
        tool_name: "agent_cmd".into(),
        args: serde_json::json!({"agent_id":"worker-1", "command":"get_state"}),
    });
    assert_eq!(
        app.conn.spinner.as_ref().unwrap().message(),
        "get_state → worker-1...",
        "agent_cmd includes command and target"
    );
    let awaiting = "Working... (Esc to interrupt)";
    app.handle_event(Event::ToolExecutionEnd {
        tool_call_id: "cmd-1".into(),
        tool_name: "agent_cmd".into(),
        result: serde_json::json!({"content":[{"type":"text","text":"done"}]}),
        is_error: false,
    });
    // Tool end keeps the spinner alive and resets it to the working message.
    assert_eq!(
        app.conn.spinner.as_ref().unwrap().message(),
        awaiting,
        "tool end keeps working msg"
    );
}

#[tokio::test]
async fn handles_response_variants() {
    let mut app = test_app().await;
    app.handle_event(Event::Response {
        id: None,
        command: "get_state".into(),
        success: true,
        data: Some(serde_json::json!({
            "model":"test-model",
            "maxContextTokens": 123,
            "workflow": {"steps": [], "progress": {"done": 0, "total": 0}}
        })),
        error: None,
    });
    assert_eq!(
        app.conn.inference.current_model.as_deref(),
        Some("test-model")
    );

    for command in ["set_model", "list_sessions", "resume_session"] {
        app.handle_event(Event::Response {
            id: None,
            command: command.into(),
            success: false,
            data: None,
            error: Some("nope".into()),
        });
    }

    app.handle_event(Event::Response {
        id: None,
        command: "get_session_stats".into(),
        success: true,
        data: Some(serde_json::json!({"messageCount": 2, "totalTokens": 10})),
        error: None,
    });
    app.handle_event(Event::Response {
        id: None,
        command: "list_sessions".into(),
        success: true,
        data: Some(serde_json::json!({"sessions": []})),
        error: None,
    });
    app.handle_event(Event::Response {
        id: None,
        command: "resume_session".into(),
        success: true,
        data: Some(serde_json::json!({"session":"cli:test"})),
        error: None,
    });
    app.handle_event(Event::Response {
        id: None,
        command: "get_messages".into(),
        success: true,
        data: Some(serde_json::json!({"messages": [{"role":"user", "content":"hi"}]})),
        error: None,
    });
}

#[tokio::test]
async fn forwarded_child_workflow_state_does_not_clobber_parent_bar() {
    let mut app = test_app().await;
    // Register a child subagent.
    app.handle_event(Event::SubagentStateChanged {
        subagents: vec![crate::protocol::client::SubagentInfoEvent {
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
            execution_backend: None,
            environment: None,
        }],
    });
    // A workflow_state forwarded up from the child (agent_id = "child", a
    // known subagent) must NOT touch the parent's own workflow bar.
    app.handle_event(Event::WorkflowState {
        agent_id: Some("child".into()),
        steps: vec![],
        progress: serde_json::json!({"done": 3, "total": 5}),
        active_issue: Some(serde_json::json!({"number": 7, "title": "child"})),
        mode: Some("active".into()),
        active_template: None,
        available_templates: None,
    });
    assert!(
        app.conn.master_session.workflow_bar.issue_number.is_none(),
        "a forwarded child event must not set the parent's workflow bar"
    );

    // The race that caused the "first loaded" flash: a forwarded event for a
    // child NOT yet registered in subagent_local must still be ignored.
    app.handle_event(Event::WorkflowState {
        agent_id: Some("unregistered-child".into()),
        steps: vec![],
        progress: serde_json::json!({"done": 1, "total": 4}),
        active_issue: Some(serde_json::json!({"number": 3, "title": "x"})),
        mode: Some("active".into()),
        active_template: None,
        available_templates: None,
    });
    assert!(
        app.conn.master_session.workflow_bar.issue_number.is_none(),
        "an unregistered child's first forwarded event must not flash the parent bar"
    );

    // The connected agent's own event (no agent_id) does update the bar.
    app.handle_event(Event::WorkflowState {
        agent_id: None,
        steps: vec![],
        progress: serde_json::json!({"done": 1, "total": 2}),
        active_issue: Some(serde_json::json!({"number": 9, "title": "parent"})),
        mode: Some("active".into()),
        active_template: None,
        available_templates: None,
    });
    assert_eq!(app.conn.master_session.workflow_bar.issue_number, Some(9));
}

#[tokio::test]
async fn named_connected_agent_own_workflow_updates_bar() {
    // When attached to a NAMED agent (e.g. a resumed session), its own
    // workflow_state carries its agent_id — it must still update the bar
    // (the old `agent_id.is_some()` guard would have wrongly dropped it).
    let mut app = test_app().await;
    app.handle_event(Event::Response {
        id: Some("init".into()),
        command: "get_state".into(),
        success: true,
        data: Some(serde_json::json!({ "sessionKey": "cli:foo" })),
        error: None,
    });
    app.handle_event(Event::WorkflowState {
        agent_id: Some("foo".into()),
        steps: vec![],
        progress: serde_json::json!({"done": 1, "total": 2}),
        active_issue: Some(serde_json::json!({"number": 11, "title": "own"})),
        mode: Some("active".into()),
        active_template: None,
        available_templates: None,
    });
    assert_eq!(
        app.conn.master_session.workflow_bar.issue_number,
        Some(11),
        "named agent's own event should update its bar"
    );
    // A descendant's forwarded event (different agent_id) must NOT.
    app.handle_event(Event::WorkflowState {
        agent_id: Some("child".into()),
        steps: vec![],
        progress: serde_json::json!({"done": 2, "total": 3}),
        active_issue: Some(serde_json::json!({"number": 22, "title": "child"})),
        mode: Some("active".into()),
        active_template: None,
        available_templates: None,
    });
    assert_eq!(
        app.conn.master_session.workflow_bar.issue_number,
        Some(11),
        "a child's forwarded event must not overwrite the named agent's bar"
    );
}

#[tokio::test]
async fn handles_subagent_workflow_and_error_events() {
    let mut app = test_app().await;
    let info = crate::protocol::client::SubagentInfoEvent {
        agent_uuid: None,
        display_name: None,
        agent_id: "a1".into(),
        status: "running".into(),
        last_tool: Some("read".into()),
        last_error: None,
        pid: 42,
        socket_path: None,
        parent_id: None,
        workflow: None,
        read_only: false,
        execution_backend: None,
        environment: None,
    };
    app.handle_event(Event::SubagentStateChanged {
        subagents: vec![info.clone()],
    });
    app.handle_event(Event::Response {
        id: None,
        command: "get_subagents".into(),
        success: true,
        data: Some(serde_json::json!({"subagents": [{
            "agentId": "a1",
            "status": "running",
            "lastTool": "read",
            "lastError": null,
            "pid": 42
        }]})),
        error: None,
    });
    app.handle_event(Event::WorkflowState {
        agent_id: None,
        steps: vec![],
        progress: serde_json::json!({"done": 0, "total": 0}),
        active_issue: Some(serde_json::json!({"number": 1, "title": "Issue"})),
        mode: Some("active".into()),
        active_template: None,
        available_templates: Some(vec![]),
    });
    app.handle_event(Event::Response {
        id: None,
        command: "agent_error".into(),
        success: false,
        data: None,
        error: Some("boom".into()),
    });
    assert!(!app.conn.agent_state.is_running());
}
#[test]
fn sanitized_arg_strips_control_chars_and_uses_fallback() {
    let args = serde_json::json!({"agent_id":"a\u{0007}b"});
    assert_eq!(sanitized_arg(&args, "agent_id", "x"), "ab");
    assert_eq!(sanitized_arg(&args, "missing", "x"), "x");
}

// ── Edge cases for tool/spawn handling (issue #729) ───────────────

#[tokio::test]
async fn update_tool_spinner_is_noop_when_spinner_none() {
    // When no spinner is active, update_tool_spinner should be a no-op
    // (no panic, no spinner magically created).
    let mut app = test_app().await;
    assert!(app.conn.spinner.is_none());
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "t1".into(),
        tool_name: "bash".into(),
        args: serde_json::json!({"command": "ls"}),
    });
    // Spinner should still be None — handle_tool_start doesn't create one.
    assert!(app.conn.spinner.is_none());
}

#[tokio::test]
async fn update_tool_spinner_formats_spawn_message() {
    let mut app = test_app().await;
    app.conn.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id": "my-agent"}),
    });
    // The spinner message should contain the agent_id.
    if let Some(ref spinner) = app.conn.spinner {
        let msg = spinner.message();
        assert!(
            msg.contains("my-agent"),
            "spawn spinner should mention agent_id: {msg}"
        );
    }
}

#[tokio::test]
async fn update_tool_spinner_formats_agent_cmd_generic_message() {
    let mut app = test_app().await;
    app.conn.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "cmd-1".into(),
        tool_name: "agent_cmd".into(),
        args: serde_json::json!({"agent_id": "worker-1", "command": "prompt"}),
    });
    if let Some(ref spinner) = app.conn.spinner {
        let msg = spinner.message();
        assert!(
            msg.contains("prompt"),
            "agent_cmd should show command: {msg}"
        );
        assert!(
            msg.contains("worker-1"),
            "agent_cmd should show agent_id: {msg}"
        );
    }
}

#[tokio::test]
async fn update_tool_spinner_formats_generic_tool_message() {
    let mut app = test_app().await;
    app.conn.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "read-1".into(),
        tool_name: "read".into(),
        args: serde_json::json!({"path": "file.txt"}),
    });
    if let Some(ref spinner) = app.conn.spinner {
        let msg = spinner.message();
        assert!(
            msg.contains("read"),
            "generic tool should show tool name: {msg}"
        );
    }
}

#[tokio::test]
async fn command_send_failure_becomes_error_notification() {
    let client = Client::disconnected_for_tests();
    let mut app = App::new(Terminal::new(), client);

    app.send_command(Command::GetState {
        agent_id: None,
        id: Some("test".into()),
    });
    let failure = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        app.command_send_failure_rx.recv(),
    )
    .await
    .expect("send failure should be routed to app")
    .expect("failure channel should stay open");

    app.handle_command_send_failure(failure);
    let rendered = app.notifications.render(120).join("\n");
    assert!(
        rendered.contains("Failed to send get_state command"),
        "notification should identify failed command without payload: {rendered}"
    );
    assert!(
        rendered.contains("disconnected"),
        "notification should include the send error: {rendered}"
    );
}
