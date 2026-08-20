use crate::domain::tool::{Tool, ToolResult};
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
use std::path::PathBuf;

fn json_response(messages: serde_json::Value) -> String {
    serde_json::json!({"success": true, "data": {"messages": messages}}).to_string()
}

#[test]
fn unrecoverable_default_get_messages_reports_are_incomplete_and_not_acknowledged() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(1);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    let shaped = tool.shape_default_get_messages_report(
        "w1",
        &json_response(serde_json::json!([
            {"role":"assistant","content":"old","ordinal":1},
            {"role":"assistant","content":"final ".repeat(10_000),"ordinal":2}
        ])),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    assert!(parsed["data"]["messages"].as_array().unwrap().is_empty());
    assert_eq!(parsed["data"]["hasMoreMessages"], true);
    assert_eq!(parsed["data"]["reportIncomplete"], true);
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content: shaped,
            is_error: false,
            image_blocks: vec![],
        },
    );
    assert_eq!(
        registry.lock().unwrap()["w1"].delivered_message_ordinal,
        Some(1)
    );

    let mut entry = registry.lock().unwrap().get("w1").unwrap().clone();
    entry.delivered_message_ordinal = Some(1);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let shaped = tool.shape_default_get_messages_report(
        "w1",
        &json_response(serde_json::json!([
            {"role":"assistant","content":"old","ordinal":1},
            {"role":"assistant","content":{"omitted":"x".repeat(20_000)},"ordinal":2}
        ])),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    assert!(parsed["data"]["messages"].as_array().unwrap().is_empty());
    assert_eq!(parsed["data"]["hasMoreMessages"], true);
    assert_eq!(parsed["data"]["reportIncomplete"], true);
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content: shaped,
            is_error: false,
            image_blocks: vec![],
        },
    );
    assert_eq!(
        registry.lock().unwrap()["w1"].delivered_message_ordinal,
        Some(1)
    );
}
