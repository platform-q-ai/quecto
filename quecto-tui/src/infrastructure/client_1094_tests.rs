//! #1094 — TUI wire contract for ranged get_message recovery.

use super::*;

#[test]
fn command_get_message_range_serializes() {
    let cmd = Command::GetMessage {
        id: Some("gm-range".into()),
        message_id: "m2".into(),
        agent_id: Some("worker".into()),
        offset: Some(4096),
        limit: Some(8192),
    };
    let json = serde_json::to_string(&cmd).unwrap();
    assert!(json.contains("\"type\":\"get_message\""));
    assert!(json.contains("\"messageId\":\"m2\""));
    assert!(json.contains("\"agent_id\":\"worker\""));
    assert!(json.contains("\"offset\":4096"));
    assert!(json.contains("\"limit\":8192"));
}
