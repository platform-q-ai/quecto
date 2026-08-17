//! #1094 — TUI wire contract for ranged get_message recovery.

use super::*;

#[test]
fn command_get_message_range_serializes() {
    let cmd = Command::GetMessage {
        id: Some("gm-range".into()),
        message_id: "m2".into(),
        agent_id: Some("worker".into()),
        tool_call_id: None,
        offset: Some(4096),
        thinking_offset: None,
        limit: Some(8192),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"get_message\""));
    assert!(json.contains("\"messageId\":\"m2\""));
    assert!(json.contains("\"agent_id\":\"worker\""));
    assert!(json.contains("\"offset\":4096"));
    assert!(json.contains("\"limit\":8192"));
}

#[test]
fn command_get_message_range_serializes_independent_thinking_cursor() {
    let cmd = Command::GetMessage {
        id: Some("gm-thinking-range".into()),
        message_id: "m3".into(),
        agent_id: None,
        tool_call_id: None,
        offset: Some(0),
        thinking_offset: Some(12288),
        limit: Some(4096),
    };
    let json = serde_json::to_value(&cmd).unwrap();
    assert_eq!(json["type"], "get_message");
    assert_eq!(json["messageId"], "m3");
    assert_eq!(json["offset"], 0);
    assert_eq!(json["thinkingOffset"], 12288);
    assert_eq!(json["limit"], 4096);
}

#[test]
fn inspection_routing_preserves_get_message_thinking_offset() {
    use crate::protocol::inspection_routing::with_inspection_agent_id;

    let routed = with_inspection_agent_id(
        &Command::GetMessage {
            id: Some("req".into()),
            message_id: "msg".into(),
            agent_id: None,
            tool_call_id: None,
            offset: Some(3),
            thinking_offset: Some(7),
            limit: Some(11),
        },
        "child-a",
        "tab0:",
    )
    .expect("get_message with thinking cursor must route");

    match routed {
        Command::GetMessage {
            agent_id,
            offset,
            thinking_offset,
            limit,
            ..
        } => {
            assert_eq!(agent_id.as_deref(), Some("child-a"));
            assert_eq!(offset, Some(3));
            assert_eq!(thinking_offset, Some(7));
            assert_eq!(limit, Some(11));
        }
        other => panic!("unexpected routed command: {other:?}"),
    }
}
