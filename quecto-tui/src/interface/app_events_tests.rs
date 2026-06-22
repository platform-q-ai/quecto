use super::*;
use crate::infrastructure::terminal::Terminal;
use tokio::io::AsyncReadExt;

async fn test_app() -> App {
    let dir = std::env::temp_dir().join(format!(
        "quecto-tui-app-events-test-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket_path = dir.join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
    });
    let client = Client::connect(&socket_path).await.unwrap();
    App::new(Terminal::new(), client)
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

#[tokio::test]
async fn handles_agent_lifecycle_and_token_events() {
    let mut app = test_app().await;
    app.handle_event(Event::AgentStart);
    assert!(app.agent_state.is_running());
    assert!(app.spinner.is_some());
    app.handle_event(Event::Token {
        token: "hello".into(),
    });
    app.handle_event(Event::TurnStart);
    app.handle_event(Event::AgentEnd { messages: vec![] });
    assert!(!app.agent_state.is_running());
    assert!(app.spinner.is_none());
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
        tool_results: vec![],
    });
    assert!(app.context_stats_requested);
    let rendered = app.footer.render(80).join("\n");
    assert!(
        rendered.contains("40/100"),
        "footer should use contextTokens: {rendered}"
    );

    let mut app = test_app().await;
    app.handle_event(Event::TurnEnd {
        message: serde_json::json!({"usage": {"total": 1}}),
        tool_results: vec![],
    });
    assert!(app.context_stats_requested);
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
        tool_results: vec![],
    });
    assert!(app.context_stats_requested);
    let rendered = app.footer.render(80).join("\n");
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

    let rendered = app.footer.render(80).join("\n");
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
    app.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id":"worker-1"}),
    });
    assert!(app.subagent_local.contains_key("worker-1"));

    app.handle_event(Event::ToolExecutionEnd {
            tool_call_id: "spawn-1".into(),
            tool_name: "spawn".into(),
            result: serde_json::json!({"content":[{"type":"text","text":"Subagent 'worker-1' is running"}]}),
            is_error: false,
        });
    assert_eq!(app.subagent_local["worker-1"].info.status, "running");

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

#[tokio::test]
async fn handles_agent_cmd_spinner_and_subagent_refresh() {
    let mut app = test_app().await;
    app.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "cmd-1".into(),
        tool_name: "agent_cmd".into(),
        args: serde_json::json!({"agent_id":"worker-1", "command":"await"}),
    });
    app.handle_event(Event::ToolExecutionEnd {
        tool_call_id: "cmd-1".into(),
        tool_name: "agent_cmd".into(),
        result: serde_json::json!({"content":[{"type":"text","text":"done"}]}),
        is_error: false,
    });
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
    assert_eq!(app.current_model.as_deref(), Some("test-model"));

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
        subagents: vec![crate::infrastructure::client::SubagentInfoEvent {
            agent_id: "child".into(),
            status: "running".into(),
            last_tool: None,
            last_error: None,
            pid: 0,
            parent_id: None,
            workflow: None,
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
        app.workflow_bar.issue_number.is_none(),
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
        app.workflow_bar.issue_number.is_none(),
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
    assert_eq!(app.workflow_bar.issue_number, Some(9));
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
        app.workflow_bar.issue_number,
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
        app.workflow_bar.issue_number,
        Some(11),
        "a child's forwarded event must not overwrite the named agent's bar"
    );
}

#[tokio::test]
async fn handles_subagent_workflow_and_error_events() {
    let mut app = test_app().await;
    let info = crate::infrastructure::client::SubagentInfoEvent {
        agent_id: "a1".into(),
        status: "running".into(),
        last_tool: Some("read".into()),
        last_error: None,
        pid: 42,
        parent_id: None,
        workflow: None,
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
    assert!(!app.agent_state.is_running());
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
    assert!(app.spinner.is_none());
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "t1".into(),
        tool_name: "bash".into(),
        args: serde_json::json!({"command": "ls"}),
    });
    // Spinner should still be None — handle_tool_start doesn't create one.
    assert!(app.spinner.is_none());
}

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
        app.subagent_local.is_empty(),
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
        app.subagent_local.contains_key("ab"),
        "control chars should be stripped from agent_id"
    );
    assert!(
        !app.subagent_local.contains_key("a\u{0007}b"),
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
    assert!(app.subagent_local.contains_key("worker-1"));
    assert_eq!(app.subagent_local["worker-1"].info.status, "starting");

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
        app.subagent_local["worker-1"].info.status, "starting",
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
        app.subagent_local["worker-1"].info.status, "starting",
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
        app.subagent_local["worker-1"].info.status, "starting",
        "error result should not mark subagent as running"
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
    assert_eq!(app.subagent_local["worker-1"].info.status, "starting");
    assert!(!app.subagent_local.contains_key("unknown-agent"));
}

#[tokio::test]
async fn update_tool_spinner_formats_spawn_message() {
    let mut app = test_app().await;
    app.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id": "my-agent"}),
    });
    // The spinner message should contain the agent_id.
    if let Some(ref spinner) = app.spinner {
        let msg = spinner.message();
        assert!(
            msg.contains("my-agent"),
            "spawn spinner should mention agent_id: {msg}"
        );
    }
}

#[tokio::test]
async fn update_tool_spinner_formats_agent_cmd_await_message() {
    let mut app = test_app().await;
    app.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "cmd-1".into(),
        tool_name: "agent_cmd".into(),
        args: serde_json::json!({"agent_id": "worker-1", "command": "await"}),
    });
    if let Some(ref spinner) = app.spinner {
        let msg = spinner.message();
        assert!(
            msg.contains("Working... (Esc to interrupt)"),
            "agent_cmd await should have stable message: {msg}"
        );
    }
}

#[tokio::test]
async fn update_tool_spinner_formats_agent_cmd_generic_message() {
    let mut app = test_app().await;
    app.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "cmd-1".into(),
        tool_name: "agent_cmd".into(),
        args: serde_json::json!({"agent_id": "worker-1", "command": "prompt"}),
    });
    if let Some(ref spinner) = app.spinner {
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
    app.spinner = Some(Spinner::new("Working"));
    app.handle_event(Event::ToolExecutionStart {
        tool_call_id: "read-1".into(),
        tool_name: "read".into(),
        args: serde_json::json!({"path": "file.txt"}),
    });
    if let Some(ref spinner) = app.spinner {
        let msg = spinner.message();
        assert!(
            msg.contains("read"),
            "generic tool should show tool name: {msg}"
        );
    }
}
