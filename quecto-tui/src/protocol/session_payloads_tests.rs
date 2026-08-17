use serde_json::json;

use super::{
    ResumeMessagesError, ResumedChatMessage, parse_resume_sessions, parse_resumed_messages,
    parse_session_stats,
};

#[test]
fn parse_session_stats_extracts_typed_values() {
    let stats = parse_session_stats(&json!({
        "sessionKey": "cli:issue-741",
        "totalMessages": 7,
        "tokens": {"input": 123, "output": 456},
        "cost": 0.125,
        "contextTokens": 3000,
        "maxContextTokens": 12000
    }));

    assert_eq!(stats.session_key, "cli:issue-741");
    assert_eq!(stats.total_messages, 7);
    assert_eq!(stats.input_tokens, 123);
    assert_eq!(stats.output_tokens, 456);
    assert_eq!(stats.cost, 0.125);
    assert_eq!(stats.context_usage, Some((3000, 12000)));
}

#[test]
fn parse_session_stats_defaults_malformed_fields() {
    let stats = parse_session_stats(&json!({"sessionKey": 42, "tokens": {"input": "bad"}}));

    assert_eq!(stats.session_key, "?");
    assert_eq!(stats.total_messages, 0);
    assert_eq!(stats.input_tokens, 0);
    assert_eq!(stats.output_tokens, 0);
    assert_eq!(stats.cost, 0.0);
    assert_eq!(stats.context_usage, None);
}

#[test]
fn parse_resume_sessions_extracts_selector_metadata() {
    let sessions = parse_resume_sessions(&json!({
        "sessions": [
            {"key": "chat-123", "title": "Fix bug", "name": "fallback", "messageCount": 12, "updatedUnixSecs": 1781980920u64},
            {"name": "No key", "updatedAt": 1700000000u64},
            {"messageCount": 99}
        ]
    }));

    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].key, "chat-123");
    assert_eq!(sessions[0].title, "Fix bug");
    assert_eq!(sessions[0].message_count, 12);
    assert_eq!(sessions[0].updated_unix_secs, Some(1781980920));
    assert_eq!(sessions[1].key, "No key");
    assert_eq!(sessions[1].title, "No key");
    assert_eq!(sessions[1].message_count, 0);
    assert_eq!(sessions[1].updated_unix_secs, Some(1700000000));
}

#[test]
fn parse_resumed_messages_keeps_only_displayable_chat_messages() {
    let messages = parse_resumed_messages(&json!({
        "messages": [
            {"role": "user", "content": "hello", "id": "u1"},
            {"role": "assistant", "content": "world", "id": "a1", "collapsed": true, "contentLength": 42},
            {"role": "assistant", "content": ""},
            {"role": "tool", "content": "hidden"}
        ]
    }))
    .expect("valid messages array should parse");

    assert_eq!(
        messages,
        vec![
            ResumedChatMessage::User {
                text: "hello".to_string(),
                id: Some("u1".to_string()),
                stub: false,
                content_len: None,
            },
            ResumedChatMessage::Assistant {
                text: "world".to_string(),
                thinking: Vec::new(),
                id: Some("a1".to_string()),
                stub: true,
                content_len: Some(42),
            },
        ]
    );
}

#[test]
fn parse_resumed_messages_preserves_visible_thinking() {
    let messages = parse_resumed_messages(&json!({
        "messages": [{"role":"assistant","content":"answer","id":"a1","visibleThinking":[{"text":"reasoning"}]}]
    }))
    .expect("valid messages array should parse");
    assert_eq!(
        messages,
        vec![ResumedChatMessage::Assistant {
            text: "answer".to_string(),
            thinking: vec!["reasoning".to_string()],
            id: Some("a1".to_string()),
            stub: false,
            content_len: None,
        }]
    );
}

#[test]
fn parse_resumed_messages_keeps_thinking_only_assistant_messages() {
    let messages = parse_resumed_messages(&json!({
        "messages": [{"role":"assistant","content":"","id":"a1","visibleThinking":[{"text":"reasoning"}]}]
    }))
    .expect("valid messages array should parse");
    assert_eq!(
        messages,
        vec![ResumedChatMessage::Assistant {
            text: String::new(),
            thinking: vec!["reasoning".to_string()],
            id: Some("a1".to_string()),
            stub: false,
            content_len: None,
        }]
    );
}

#[test]
fn parse_resumed_messages_preserves_tool_calls_and_results() {
    let messages = parse_resumed_messages(&json!({
        "messages": [
            {
                "role": "assistant",
                "content": "",
                "id": "a-tools",
                "toolCalls": [
                    {
                        "id": "call-1",
                        "function": {
                            "name": "bash",
                            "arguments": r#"{"command":"printf restored"}"#
                        }
                    }
                ]
            },
            {
                "role": "tool",
                "toolCallId": "call-1",
                "toolName": "bash",
                "content": "restored output",
                "isError": false
            },
            {"role": "assistant", "content": "after tool"}
        ]
    }))
    .expect("valid messages array should parse");

    assert_eq!(
        messages,
        vec![
            ResumedChatMessage::ToolCall {
                tool_call_id: "call-1".to_string(),
                tool_name: "bash".to_string(),
                args: r#"{"command":"printf restored"}"#.to_string(),
            },
            ResumedChatMessage::ToolResult {
                tool_call_id: "call-1".to_string(),
                tool_name: Some("bash".to_string()),
                content: "restored output".to_string(),
                is_error: false,
            },
            ResumedChatMessage::Assistant {
                text: "after tool".to_string(),
                thinking: Vec::new(),
                id: None,
                stub: false,
                content_len: None,
            },
        ]
    );
}

#[test]
fn parse_resumed_messages_preserves_multiple_pending_error_and_snake_case_tools() {
    let messages = parse_resumed_messages(&json!({
        "messages": [
            {
                "role": "assistant",
                "content": "",
                "tool_calls": [
                    {"id": "call-1", "function": {"name": "bash", "arguments": r#"{"command":"first"}"#}},
                    {"id": "call-2", "function": {"name": "read", "arguments": r#"{"path":"second.txt"}"#}}
                ]
            },
            {"role": "tool", "tool_call_id": "call-1", "tool_name": "bash", "content": "boom", "is_error": true}
        ]
    }))
    .expect("valid messages array should parse");

    assert_eq!(
        messages,
        vec![
            ResumedChatMessage::ToolCall {
                tool_call_id: "call-1".to_string(),
                tool_name: "bash".to_string(),
                args: r#"{"command":"first"}"#.to_string(),
            },
            ResumedChatMessage::ToolCall {
                tool_call_id: "call-2".to_string(),
                tool_name: "read".to_string(),
                args: r#"{"path":"second.txt"}"#.to_string(),
            },
            ResumedChatMessage::ToolResult {
                tool_call_id: "call-1".to_string(),
                tool_name: Some("bash".to_string()),
                content: "boom".to_string(),
                is_error: true,
            },
        ]
    );
}

#[test]
fn parse_resumed_messages_keeps_assistant_text_before_tool_calls() {
    let messages = parse_resumed_messages(&json!({
        "messages": [{
            "role": "assistant",
            "content": "I will inspect it",
            "toolCalls": [{"id": "call-1", "function": {"name": "read", "arguments": r#"{"path":"src/lib.rs"}"#}}]
        }]
    }))
    .expect("valid messages array should parse");

    assert_eq!(
        messages,
        vec![
            ResumedChatMessage::Assistant {
                text: "I will inspect it".to_string(),
                thinking: Vec::new(),
                id: None,
                stub: false,
                content_len: None,
            },
            ResumedChatMessage::ToolCall {
                tool_call_id: "call-1".to_string(),
                tool_name: "read".to_string(),
                args: r#"{"path":"src/lib.rs"}"#.to_string(),
            },
        ]
    );
}

#[test]
fn parse_resumed_messages_rejects_missing_messages() {
    assert_eq!(
        parse_resumed_messages(&json!({})),
        Err(ResumeMessagesError::MissingMessages)
    );
}

#[test]
fn parse_resumed_messages_rejects_non_array_messages() {
    assert_eq!(
        parse_resumed_messages(&json!({"messages": "bad"})),
        Err(ResumeMessagesError::MalformedMessages)
    );
}

/// Sub-agent notes travel as user-role turns so the model answers them (#1338),
/// but they are harness status, not operator input — a resumed transcript must
/// not redraw them as messages the user typed.
#[test]
fn resumed_messages_skip_subagent_notes() {
    let messages = parse_resumed_messages(&json!({
        "messages": [
            {"role": "user", "content": "write a poem"},
            {"role": "user", "content": "<subagent_notification source=\"spawn_tool\" agent_id=\"poet\" sequence=\"1\">\nSub-agent 'poet' ended a turn (status: idle).\n</subagent_notification>"},
            {"role": "assistant", "content": "done"}
        ]
    }))
    .expect("payload should parse");

    let user_texts: Vec<&str> = messages
        .iter()
        .filter_map(|m| match m {
            ResumedChatMessage::User { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        user_texts,
        vec!["write a poem"],
        "the injected sub-agent note must not resume as a user message"
    );
}
