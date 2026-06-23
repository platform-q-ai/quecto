use serde_json::json;

use super::{
    ResumedChatMessage, parse_resume_sessions, parse_resumed_messages, parse_session_stats,
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
            {"role": "user", "content": "hello"},
            {"role": "assistant", "content": "world"},
            {"role": "assistant", "content": ""},
            {"role": "tool", "content": "hidden"}
        ]
    }));

    assert_eq!(
        messages,
        vec![
            ResumedChatMessage::User("hello".to_string()),
            ResumedChatMessage::Assistant("world".to_string()),
        ]
    );
}
