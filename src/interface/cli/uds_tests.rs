/// Unit tests for the UDS agent loop — session state, stats, and dispatch helpers.
///
/// This file is compiled as `mod tests` inside `uds.rs`, so `super` = `uds`.
/// Session-layer types (`AgentSession`, `compute_session_stats`, etc.) are
/// re-exported from `uds` via `uds_session`.  Protocol types are imported from
/// `cli::protocol`.
use super::*;
use crate::interface::cli::protocol::*;

// ─── AgentSession unit tests ───────────────────────────────────────────────────

#[test]
fn test_initial_state_not_streaming() {
    let session = AgentSession::new("gpt-5".to_string(), "cli:test".to_string());
    assert!(!session.is_streaming());
}

#[test]
fn test_set_model_changes_model() {
    let mut session = AgentSession::new("gpt-5".to_string(), "cli:test".to_string());
    session.set_model("gpt-5-mini".to_string());
    assert_eq!(session.model(), "gpt-5-mini");
}

#[test]
fn test_session_state_snapshot() {
    let session = AgentSession::new("gpt-5".to_string(), "cli:my".to_string());
    let state = session.state_snapshot(4);
    assert_eq!(state.model, "gpt-5");
    assert!(!state.is_streaming);
    assert_eq!(state.session_key, "cli:my");
    assert_eq!(state.message_count, 4);
    assert_eq!(state.pending_message_count, 0);
}

#[test]
fn test_pending_message_count_after_enqueue() {
    let mut session = AgentSession::new("m".to_string(), "k".to_string());
    session.enqueue_pending("first".to_string());
    session.enqueue_pending("second".to_string());
    let state = session.state_snapshot(0);
    assert_eq!(state.pending_message_count, 2);
}

#[test]
fn test_drain_pending_messages() {
    let mut session = AgentSession::new("m".to_string(), "k".to_string());
    session.enqueue_pending("a".to_string());
    session.enqueue_pending("b".to_string());
    let drained = session.drain_pending();
    assert_eq!(drained, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(session.state_snapshot(0).pending_message_count, 0);
}

// ─── compute_session_stats ───────────────────────────────────────────────────

#[test]
fn test_stats_empty_messages() {
    use crate::domain::message::Message;
    let msgs: Vec<Message> = vec![];
    let stats = compute_session_stats("cli:test", &msgs);
    assert_eq!(stats.user_messages, 0);
    assert_eq!(stats.assistant_messages, 0);
    assert_eq!(stats.tool_calls, 0);
    assert_eq!(stats.tool_results, 0);
    assert_eq!(stats.total_messages, 0);
    assert_eq!(stats.cost, 0.0);
}

#[test]
fn test_stats_counts_user_and_assistant() {
    use crate::domain::message::Message;
    let msgs = vec![
        Message::user("hello".to_string()),
        Message::assistant("hi".to_string(), vec![]),
        Message::user("bye".to_string()),
    ];
    let stats = compute_session_stats("k", &msgs);
    assert_eq!(stats.user_messages, 2);
    assert_eq!(stats.assistant_messages, 1);
    assert_eq!(stats.total_messages, 3);
}

#[test]
fn test_stats_counts_tool_calls_and_results() {
    use crate::domain::message::{Message, ToolCall};
    let msgs = vec![
        Message::user("hi".to_string()),
        Message::assistant(
            String::new(),
            vec![
                ToolCall {
                    id: "c1".to_string(),
                    name: "bash".to_string(),
                    arguments: "{}".to_string(),
                },
                ToolCall {
                    id: "c2".to_string(),
                    name: "read".to_string(),
                    arguments: "{}".to_string(),
                },
            ],
        ),
        Message::tool("c1".to_string(), "out1".to_string()),
        Message::tool("c2".to_string(), "out2".to_string()),
    ];
    let stats = compute_session_stats("k", &msgs);
    assert_eq!(stats.tool_calls, 2);
    assert_eq!(stats.tool_results, 2);
    assert_eq!(stats.total_messages, 4);
}

#[test]
fn test_stats_tokens_zeroed_without_usage_on_message() {
    // Token usage is not stored on Message objects; stats returns zeroed tokens.
    use crate::domain::message::Message;
    let msgs = vec![
        Message::user("hi".to_string()),
        Message::assistant("reply".to_string(), vec![]),
    ];
    let stats = compute_session_stats("k", &msgs);
    assert_eq!(stats.tokens.input, 0);
    assert_eq!(stats.tokens.output, 0);
    assert_eq!(stats.tokens.total, 0);
    assert_eq!(stats.cost, 0.0);
}

// ─── parse_command_line ──────────────────────────────────────────────────────────

#[test]
fn test_parse_valid_prompt_line() {
    let line = r#"{"type":"prompt","message":"hello"}"#;
    let result = parse_command_line(line);
    assert!(result.is_ok());
    matches!(result.unwrap(), AgentCommand::Prompt { .. });
}

#[test]
fn test_parse_invalid_json_returns_err() {
    let result = parse_command_line("not json{");
    assert!(result.is_err());
}

#[test]
fn test_parse_unknown_type_returns_err() {
    let result = parse_command_line(r#"{"type":"unknown"}"#);
    assert!(result.is_err());
}

#[test]
fn test_parse_empty_line_returns_err() {
    let result = parse_command_line("");
    assert!(result.is_err());
}

#[test]
fn test_parse_abort_command() {
    let line = r#"{"type":"abort","id":"ab-1"}"#;
    let cmd = parse_command_line(line).unwrap();
    assert_eq!(cmd.id(), Some("ab-1"));
    assert_eq!(cmd.type_name(), "abort");
}

#[test]
fn test_parse_set_model_command() {
    let line = r#"{"type":"set_model","model":"gpt-5-mini"}"#;
    let cmd = parse_command_line(line).unwrap();
    match cmd {
        AgentCommand::SetModel {
            model,
            provider,
            model_id,
            ..
        } => {
            assert_eq!(model.as_deref(), Some("gpt-5-mini"));
            assert!(provider.is_none());
            assert!(model_id.is_none());
        }
        _ => panic!("expected SetModel"),
    }
}

#[test]
fn test_parse_set_model_provider_and_model_id_command() {
    let line = r#"{"type":"set_model","provider":"openai-codex","modelId":"gpt-5.3-codex"}"#;
    let cmd = parse_command_line(line).unwrap();
    match cmd {
        AgentCommand::SetModel {
            model,
            provider,
            model_id,
            ..
        } => {
            assert!(model.is_none());
            assert_eq!(provider.as_deref(), Some("openai-codex"));
            assert_eq!(model_id.as_deref(), Some("gpt-5.3-codex"));
        }
        _ => panic!("expected SetModel"),
    }
}

#[test]
fn test_stats_user_plus_assistant_equals_exchange() {
    // After 1 user + 1 assistant message, counts should be 1 and 1.
    let messages = vec![
        Message::user("hello"),
        Message::assistant("hi there", vec![]),
    ];
    let stats = compute_session_stats("cli:test", &messages);
    assert_eq!(stats.user_messages, 1);
    assert_eq!(stats.assistant_messages, 1);
    assert_eq!(stats.total_messages, 2);
    assert_eq!(stats.tool_calls, 0);
}

#[test]
fn test_parse_error_response_uses_parse_error_command() {
    // Malformed JSON must produce a response with command == "parse_error".
    let result = parse_command_line("{{bad json");
    assert!(result.is_err());
    let err_msg = result.unwrap_err();
    // The error message should describe a parse failure.
    assert!(
        err_msg.contains("parse") || err_msg.contains("JSON") || err_msg.contains("expected"),
        "unexpected error: {err_msg}"
    );
}

#[test]
fn test_agent_event_parse_error_response_shape() {
    // Verify that the parse error event we emit conforms to the Response shape.
    let ev = AgentEvent::Response {
        id: None,
        command: "parse_error".to_string(),
        success: false,
        data: None,
        error: Some("parse error: invalid JSON".to_string()),
    };
    let json = ev.to_json_line();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["type"], "response");
    assert_eq!(v["command"], "parse_error");
    assert_eq!(v["success"], false);
    assert!(v["error"].is_string());
}

#[test]
fn test_enqueue_pending_respects_cap() {
    let mut session = AgentSession::new("model".into(), "key".into());
    for i in 0..AgentSession::MAX_PENDING + 10 {
        session.enqueue_pending(format!("msg-{i}"));
    }
    let drained = session.drain_pending();
    assert_eq!(
        drained.len(),
        AgentSession::MAX_PENDING,
        "should cap at MAX_PENDING"
    );
}

#[test]
fn test_resolve_set_model_target_from_legacy_model_field() {
    let model = resolve_set_model_target(Some("gpt-5-mini".into()), None, None).unwrap();
    assert_eq!(model, "gpt-5-mini");
}

#[test]
fn test_resolve_set_model_target_from_provider_and_model_id() {
    let model = resolve_set_model_target(
        None,
        Some("openai-codex".into()),
        Some("gpt-5.3-codex".into()),
    )
    .unwrap();
    assert_eq!(model, "openai-codex/gpt-5.3-codex");
}

#[test]
fn test_set_model_is_reflected_in_state_snapshot() {
    let mut session = AgentSession::new("gpt-4".into(), "cli:test".into());
    session.set_model("claude-opus-4-5".into());
    let snap = session.state_snapshot(0);
    assert_eq!(snap.model, "claude-opus-4-5");
}

// ─── get_messages_tail ───────────────────────────────────────────────────────

#[test]
fn test_parse_get_messages_tail_line() {
    let line = r#"{"type":"get_messages_tail","count":3}"#;
    let cmd = parse_command_line(line).unwrap();
    match cmd {
        AgentCommand::GetMessagesTail { count, .. } => assert_eq!(count, 3),
        _ => panic!("expected GetMessagesTail"),
    }
}

#[test]
fn test_get_messages_tail_type_name() {
    let cmd = AgentCommand::GetMessagesTail { id: None, count: 5 };
    assert_eq!(cmd.type_name(), "get_messages_tail");
}

#[test]
fn test_get_messages_tail_returns_last_n_in_order() {
    // Build 5 user messages and request tail of 3 — should get last 3 in original order.
    let messages: Vec<Message> = (0..5).map(|i| Message::user(format!("msg{i}"))).collect();
    let data = messages_tail_json(&messages, 3);
    let arr = data["messages"].as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["content"], "msg2");
    assert_eq!(arr[1]["content"], "msg3");
    assert_eq!(arr[2]["content"], "msg4");
}

#[test]
fn test_messages_tail_json_empty_history() {
    let messages: Vec<Message> = vec![];
    let data = messages_tail_json(&messages, 5);
    assert!(data["messages"].as_array().unwrap().is_empty());
}

#[test]
fn test_messages_tail_json_count_zero() {
    let messages: Vec<Message> = (0..3).map(|i| Message::user(format!("m{i}"))).collect();
    let data = messages_tail_json(&messages, 0);
    assert!(data["messages"].as_array().unwrap().is_empty());
}

#[test]
fn test_messages_tail_json_count_exceeds_history() {
    let messages: Vec<Message> = (0..2).map(|i| Message::user(format!("m{i}"))).collect();
    let data = messages_tail_json(&messages, 100);
    assert_eq!(data["messages"].as_array().unwrap().len(), 2);
}

// ─── system prompt injection ─────────────────────────────────────────────────

#[test]
fn test_inject_system_prompt_prepends_system_message() {
    let mut messages: Vec<Message> = vec![Message::user("hello")];
    inject_system_prompt(&mut messages, "Be helpful.");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, crate::domain::message::Role::System);
    assert_eq!(messages[0].content, "Be helpful.");
    assert_eq!(messages[1].content, "hello");
}

#[test]
fn test_inject_system_prompt_empty_string_is_noop() {
    let mut messages: Vec<Message> = vec![Message::user("hello")];
    inject_system_prompt(&mut messages, "");
    assert_eq!(messages.len(), 1);
}

#[test]
fn test_inject_system_prompt_skips_when_system_already_present() {
    let mut messages: Vec<Message> =
        vec![Message::system("Existing system."), Message::user("hello")];
    inject_system_prompt(&mut messages, "New system.");
    // Should not prepend — existing system message already present.
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "Existing system.");
}

#[test]
fn test_remove_system_prompt_removes_first_system_message() {
    let mut messages: Vec<Message> = vec![
        Message::system("Be helpful."),
        Message::user("hello"),
        Message::assistant("hi", vec![]),
    ];
    remove_injected_system_prompt(&mut messages, "Be helpful.");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "hello");
}

#[test]
fn test_remove_system_prompt_noop_when_empty() {
    let mut messages: Vec<Message> = vec![Message::system("Keep."), Message::user("hello")];
    remove_injected_system_prompt(&mut messages, "");
    assert_eq!(messages.len(), 2);
}

#[test]
fn test_remove_system_prompt_noop_when_content_differs() {
    let mut messages: Vec<Message> = vec![Message::system("Different."), Message::user("hello")];
    remove_injected_system_prompt(&mut messages, "Be helpful.");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "Different.");
}

// ─── Socket permission tests ──────────────────────────────────────────────────

#[test]
fn test_bind_socket_mode_is_0600() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let socket_path = dir.path().join("test-mode.sock");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let listener = rt
        .block_on(async { bind_secure_socket(&socket_path) })
        .expect("bind failed");
    let mode = std::fs::metadata(&socket_path)
        .expect("stat")
        .permissions()
        .mode();
    drop(listener);

    assert_eq!(
        mode & 0o777,
        0o600,
        "expected socket mode 0600, got {:04o}",
        mode & 0o777
    );
}

#[test]
fn test_reap_stale_sockets_removes_old_files() {
    let dir = tempfile::tempdir().expect("tempdir");

    // Create a file that looks like a stale quecto socket.
    let stale = dir.path().join("quecto-agent-deadbeef.sock");
    std::fs::File::create(&stale).expect("create stale socket");

    // Create a file that does NOT match the prefix — should be left alone.
    let other = dir.path().join("other.sock");
    std::fs::File::create(&other).expect("create other");

    // Reap with max_age = 0 so everything is "stale".
    reap_stale_sockets(dir.path(), std::time::Duration::ZERO);

    assert!(
        !stale.exists(),
        "stale quecto socket should have been removed"
    );
    assert!(other.exists(), "non-quecto socket should be left alone");
}

#[test]
fn test_reap_stale_sockets_leaves_fresh_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fresh = dir.path().join("quecto-agent-fresh.sock");
    std::fs::File::create(&fresh).expect("create fresh socket");

    // Reap with max_age = 24h — a file just created is not stale.
    reap_stale_sockets(dir.path(), std::time::Duration::from_secs(86_400));

    assert!(fresh.exists(), "fresh socket should not be removed");
}

// ─── forward_progress_event forwards ToolStarted/ToolFinished (#318) ─────────

#[tokio::test]
async fn test_forward_progress_event_emits_tool_started_with_tool_call_id() {
    use crate::domain::agent::AgentProgressEvent;
    use crate::interface::cli::uds_cancel::forward_progress_event;

    let mut buf = Vec::new();
    let ev = AgentProgressEvent::ToolStarted {
        tool_call_id: "call_abc".to_string(),
        name: "bash".to_string(),
        arguments: r#"{"command":"echo hi"}"#.to_string(),
    };
    forward_progress_event(ev, &mut buf).await;

    let output = String::from_utf8(buf).unwrap();
    assert!(
        output.contains("tool_execution_start"),
        "expected tool_execution_start event, got: {output}"
    );
    assert!(
        output.contains("call_abc"),
        "expected tool_call_id 'call_abc', got: {output}"
    );
    assert!(
        output.contains("bash"),
        "expected tool_name 'bash', got: {output}"
    );
}

#[tokio::test]
async fn test_forward_progress_event_emits_tool_finished_with_tool_call_id() {
    use crate::domain::agent::AgentProgressEvent;
    use crate::interface::cli::uds_cancel::forward_progress_event;

    let mut buf = Vec::new();
    let ev = AgentProgressEvent::ToolFinished {
        tool_call_id: "call_xyz".to_string(),
        name: "read".to_string(),
        arguments: r#"{"path":"a.txt"}"#.to_string(),
        duration_ms: 42,
        is_error: false,
    };
    forward_progress_event(ev, &mut buf).await;

    let output = String::from_utf8(buf).unwrap();
    assert!(
        output.contains("tool_execution_end"),
        "expected tool_execution_end event, got: {output}"
    );
    assert!(
        output.contains("call_xyz"),
        "expected tool_call_id 'call_xyz', got: {output}"
    );
    assert!(
        output.contains("read"),
        "expected tool_name 'read', got: {output}"
    );
}
