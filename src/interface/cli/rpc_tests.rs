use super::*;
/// Unit tests for the RPC agent loop.
///
/// These tests exercise the RPC session state management, stats computation,
/// model switching, and event emission — all without real I/O or a real agent.
// This file is compiled as `mod tests` inside `rpc.rs`, so `super` = `rpc`.
// `rpc_types` lives as a sibling at `cli::rpc_types`.
use crate::interface::cli::rpc_types::*;

// ─── RpcSession unit tests ───────────────────────────────────────────────────

#[test]
fn test_initial_state_not_streaming() {
    let session = RpcSession::new("gpt-5".to_string(), "cli:test".to_string());
    assert!(!session.is_streaming());
}

#[test]
fn test_set_model_changes_model() {
    let mut session = RpcSession::new("gpt-5".to_string(), "cli:test".to_string());
    session.set_model("gpt-5-mini".to_string());
    assert_eq!(session.model(), "gpt-5-mini");
}

#[test]
fn test_session_state_snapshot() {
    let session = RpcSession::new("gpt-5".to_string(), "cli:my".to_string());
    let state = session.state_snapshot(4);
    assert_eq!(state.model, "gpt-5");
    assert!(!state.is_streaming);
    assert_eq!(state.session_key, "cli:my");
    assert_eq!(state.message_count, 4);
    assert_eq!(state.pending_message_count, 0);
}

#[test]
fn test_pending_message_count_after_enqueue() {
    let mut session = RpcSession::new("m".to_string(), "k".to_string());
    session.enqueue_pending("first".to_string());
    session.enqueue_pending("second".to_string());
    let state = session.state_snapshot(0);
    assert_eq!(state.pending_message_count, 2);
}

#[test]
fn test_drain_pending_messages() {
    let mut session = RpcSession::new("m".to_string(), "k".to_string());
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

// ─── parse_rpc_line ──────────────────────────────────────────────────────────

#[test]
fn test_parse_valid_prompt_line() {
    let line = r#"{"type":"prompt","message":"hello"}"#;
    let result = parse_rpc_line(line);
    assert!(result.is_ok());
    matches!(result.unwrap(), RpcCommand::Prompt { .. });
}

#[test]
fn test_parse_invalid_json_returns_err() {
    let result = parse_rpc_line("not json{");
    assert!(result.is_err());
}

#[test]
fn test_parse_unknown_type_returns_err() {
    let result = parse_rpc_line(r#"{"type":"unknown"}"#);
    assert!(result.is_err());
}

#[test]
fn test_parse_empty_line_returns_err() {
    let result = parse_rpc_line("");
    assert!(result.is_err());
}

#[test]
fn test_parse_abort_command() {
    let line = r#"{"type":"abort","id":"ab-1"}"#;
    let cmd = parse_rpc_line(line).unwrap();
    assert_eq!(cmd.id(), Some("ab-1"));
    assert_eq!(cmd.type_name(), "abort");
}

#[test]
fn test_parse_set_model_command() {
    let line = r#"{"type":"set_model","model":"gpt-5-mini"}"#;
    let cmd = parse_rpc_line(line).unwrap();
    match cmd {
        RpcCommand::SetModel {
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
    let cmd = parse_rpc_line(line).unwrap();
    match cmd {
        RpcCommand::SetModel {
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
    let result = parse_rpc_line("{{bad json");
    assert!(result.is_err());
    let err_msg = result.unwrap_err();
    // The error message should describe a parse failure.
    assert!(
        err_msg.contains("parse") || err_msg.contains("JSON") || err_msg.contains("expected"),
        "unexpected error: {err_msg}"
    );
}

#[test]
fn test_rpc_event_parse_error_response_shape() {
    // Verify that the parse error event we emit conforms to the Response shape.
    let ev = RpcEvent::Response {
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
    let mut session = RpcSession::new("model".into(), "key".into());
    for i in 0..RpcSession::MAX_PENDING + 10 {
        session.enqueue_pending(format!("msg-{i}"));
    }
    let drained = session.drain_pending();
    assert_eq!(
        drained.len(),
        RpcSession::MAX_PENDING,
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
    let mut session = RpcSession::new("gpt-4".into(), "cli:test".into());
    session.set_model("claude-opus-4-5".into());
    let snap = session.state_snapshot(0);
    assert_eq!(snap.model, "claude-opus-4-5");
}

// ─── get_messages_tail ───────────────────────────────────────────────────────

#[test]
fn test_parse_get_messages_tail_line() {
    let line = r#"{"type":"get_messages_tail","count":3}"#;
    let cmd = parse_rpc_line(line).unwrap();
    match cmd {
        RpcCommand::GetMessagesTail { count, .. } => assert_eq!(count, 3),
        _ => panic!("expected GetMessagesTail"),
    }
}

#[test]
fn test_get_messages_tail_type_name() {
    let cmd = RpcCommand::GetMessagesTail { id: None, count: 5 };
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
    inject_system_prompt(&mut messages, Some("Be helpful."));
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, crate::domain::message::Role::System);
    assert_eq!(messages[0].content, "Be helpful.");
    assert_eq!(messages[1].content, "hello");
}

#[test]
fn test_inject_system_prompt_none_is_noop() {
    let mut messages: Vec<Message> = vec![Message::user("hello")];
    inject_system_prompt(&mut messages, None);
    assert_eq!(messages.len(), 1);
}

#[test]
fn test_inject_system_prompt_empty_string_is_noop() {
    let mut messages: Vec<Message> = vec![Message::user("hello")];
    inject_system_prompt(&mut messages, Some(""));
    assert_eq!(messages.len(), 1);
}

#[test]
fn test_inject_system_prompt_skips_when_system_already_present() {
    let mut messages: Vec<Message> =
        vec![Message::system("Existing system."), Message::user("hello")];
    inject_system_prompt(&mut messages, Some("New system."));
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
    remove_injected_system_prompt(&mut messages, Some("Be helpful."));
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "hello");
}

#[test]
fn test_remove_system_prompt_noop_when_none() {
    let mut messages: Vec<Message> = vec![Message::system("Keep."), Message::user("hello")];
    remove_injected_system_prompt(&mut messages, None);
    assert_eq!(messages.len(), 2);
}

#[test]
fn test_remove_system_prompt_noop_when_content_differs() {
    let mut messages: Vec<Message> = vec![Message::system("Different."), Message::user("hello")];
    remove_injected_system_prompt(&mut messages, Some("Be helpful."));
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "Different.");
}
