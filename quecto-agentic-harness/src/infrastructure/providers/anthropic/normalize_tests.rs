use super::*;
use crate::domain::message::{Message, Role, StopReason, ToolCall};

// --- is_valid_tool_call_id / normalize_tool_call_id ---

#[test]
fn valid_id_returns_none() {
    assert!(normalize_tool_call_id("abc123_-XYZ").is_none());
}

#[test]
fn invalid_chars_replaced() {
    let norm = normalize_tool_call_id("call.123!@#").unwrap();
    assert!(
        norm.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    );
}

#[test]
fn long_id_truncated() {
    let long = "a".repeat(100);
    let norm = normalize_tool_call_id(&long).unwrap();
    assert!(norm.len() <= 64);
}

#[test]
fn empty_id_is_valid() {
    assert!(normalize_tool_call_id("").is_none());
}

// --- normalize_messages ---

#[test]
fn normal_messages_borrowed() {
    let msgs = vec![
        Message::user("hello".to_string()),
        Message::assistant("hi".to_string(), vec![]),
    ];
    let normalized = normalize_messages(&msgs);
    assert_eq!(normalized.len(), 2);
    assert!(matches!(normalized[0], std::borrow::Cow::Borrowed(_)));
}

#[test]
fn error_assistant_turn_dropped() {
    let mut assistant = Message::assistant("partial".to_string(), vec![]);
    assistant.stop_reason = Some(StopReason::Error);
    let msgs = vec![Message::user("hello".to_string()), assistant];
    let normalized = normalize_messages(&msgs);
    assert_eq!(normalized.len(), 1);
    assert_eq!(normalized[0].role, Role::User);
}

#[test]
fn aborted_assistant_turn_dropped() {
    let mut assistant = Message::assistant("partial".to_string(), vec![]);
    assistant.stop_reason = Some(StopReason::Aborted);
    let tc = ToolCall {
        id: "call-1".to_string(),
        name: "bash".to_string(),
        arguments: "{}".to_string(),
    };
    assistant.tool_calls = vec![tc];
    let mut tool_result = Message::tool("call-1", "result");
    tool_result.tool_call_id = Some("call-1".to_string());
    let msgs = vec![Message::user("hello".to_string()), assistant, tool_result];
    let normalized = normalize_messages(&msgs);
    // Both the aborted assistant and its orphaned tool result should be dropped
    assert_eq!(normalized.len(), 1);
}

#[test]
fn invalid_tool_call_id_normalized() {
    let mut assistant = Message::assistant("calling".to_string(), vec![]);
    assistant.tool_calls = vec![ToolCall {
        id: "call.123!@#".to_string(),
        name: "bash".to_string(),
        arguments: "{}".to_string(),
    }];
    let msgs = vec![assistant];
    let normalized = normalize_messages(&msgs);
    assert_eq!(normalized.len(), 1);
    let tc_id = &normalized[0].tool_calls[0].id;
    assert!(
        tc_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    );
}

// --- collect_tool_use_ids / collect_tool_result_ids ---

#[test]
fn collect_tool_use_ids_finds_all() {
    let msgs = vec![serde_json::json!({
        "role": "assistant",
        "content": [
            {"type": "tool_use", "id": "call-1", "name": "bash", "input": {}},
            {"type": "tool_use", "id": "call-2", "name": "read", "input": {}},
        ]
    })];
    let ids = collect_tool_use_ids(&msgs);
    assert_eq!(ids, vec!["call-1", "call-2"]);
}

#[test]
fn collect_tool_use_ids_empty() {
    let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];
    assert!(collect_tool_use_ids(&msgs).is_empty());
}

#[test]
fn collect_tool_result_ids_finds_all() {
    let msgs = vec![serde_json::json!({
        "role": "user",
        "content": [
            {"type": "tool_result", "tool_use_id": "call-1", "content": "ok"},
            {"type": "tool_result", "tool_use_id": "call-2", "content": "ok"},
        ]
    })];
    let ids = collect_tool_result_ids(&msgs);
    assert!(ids.contains("call-1"));
    assert!(ids.contains("call-2"));
}

// --- synthetic_tool_result ---

#[test]
fn synthetic_result_has_required_fields() {
    let result = synthetic_tool_result("call-42".to_string());
    assert_eq!(result["type"], "tool_result");
    assert_eq!(result["tool_use_id"], "call-42");
    assert_eq!(result["is_error"], true);
}

// --- inject_orphaned_tool_results ---

#[test]
fn inject_no_orphans_is_noop() {
    let mut msgs = vec![
        serde_json::json!({
            "role": "assistant",
            "content": [{"type": "tool_use", "id": "call-1", "name": "bash", "input": {}}]
        }),
        serde_json::json!({
            "role": "user",
            "content": [{"type": "tool_result", "tool_use_id": "call-1", "content": "ok"}]
        }),
    ];
    let len_before = msgs.len();
    inject_orphaned_tool_results(&mut msgs);
    assert_eq!(msgs.len(), len_before);
}

#[test]
fn inject_orphaned_creates_synthetic() {
    let mut msgs = vec![serde_json::json!({
        "role": "assistant",
        "content": [{"type": "tool_use", "id": "orphan-1", "name": "bash", "input": {}}]
    })];
    inject_orphaned_tool_results(&mut msgs);
    // Should have added a user message with synthetic tool_result
    assert_eq!(msgs.len(), 2);
    let content = msgs[1]["content"].as_array().unwrap();
    assert_eq!(content[0]["tool_use_id"], "orphan-1");
    assert_eq!(content[0]["is_error"], true);
}

#[test]
fn inject_orphaned_appends_to_existing_user_tool_results() {
    let mut msgs = vec![
        serde_json::json!({
            "role": "assistant",
            "content": [
                {"type": "tool_use", "id": "call-1", "name": "a", "input": {}},
                {"type": "tool_use", "id": "call-2", "name": "b", "input": {}},
            ]
        }),
        serde_json::json!({
            "role": "user",
            "content": [{"type": "tool_result", "tool_use_id": "call-1", "content": "ok"}]
        }),
    ];
    inject_orphaned_tool_results(&mut msgs);
    // Should have appended synthetic result to existing user message
    assert_eq!(msgs.len(), 2);
    let content = msgs[1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2); // original + synthetic
    assert_eq!(content[1]["tool_use_id"], "call-2");
}
