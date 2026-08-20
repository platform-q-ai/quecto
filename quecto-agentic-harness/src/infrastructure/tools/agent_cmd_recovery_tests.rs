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
    assert_eq!(parsed["data"]["messageContentTruncated"], false);
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
    assert_eq!(parsed["data"]["messageContentTruncated"], false);
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

#[test]
fn recoverable_truncated_default_report_metadata_maps_to_get_message_command() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(1);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    let message_id = "00000000-0000-0000-0000-000000000002";
    let full = "final ".repeat(10_000);
    let shaped = tool.shape_default_get_messages_report(
        "w1",
        &json_response(serde_json::json!([
            {"role":"assistant","content":"old","ordinal":1},
            {"id":message_id,"role":"assistant","content":full,"ordinal":2}
        ])),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    let reported = &parsed["data"]["messages"].as_array().unwrap()[0];
    let recovery = &reported["contentRecovery"];
    assert_eq!(recovery["command"], "get_message");
    assert_eq!(recovery["messageId"], message_id);
    let offset = recovery["offset"].as_u64().expect("recovery offset") as usize;
    assert!(offset > 0);

    let (_agent_id, cmd, command) = tool
        .parse_and_build(
            &serde_json::json!({
                "agent_id":"w1",
                "command": recovery["command"],
                "messageId": recovery["messageId"],
                "offset": recovery["offset"],
                "limit": 64
            })
            .to_string(),
        )
        .unwrap();
    assert_eq!(command, "get_message");
    let cmd: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(cmd["type"], "get_message");
    assert_eq!(cmd["messageId"], message_id);
    assert_eq!(cmd["offset"], offset);
    assert_eq!(&full[offset..offset + 64], &("final ".repeat(10) + "fina"));
}

#[tokio::test]
async fn agent_cmd_get_message_recovers_content_from_metadata_through_nested_route() {
    use tokio::io::BufReader;
    let registry = new_registry();
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("ancestor.sock");
    let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let payload =
            quecto_line_io::read_frame(&mut reader, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
                .await
                .unwrap()
                .unwrap();
        let cmd: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(cmd["type"], "get_message");
        assert_eq!(cmd["agent_id"], "grandchild");
        assert_eq!(cmd["messageId"], "m-final");
        assert_eq!(cmd["offset"], 12);
        assert_eq!(cmd["limit"], 5);
        let reply = serde_json::json!({
            "type":"response",
            "id": cmd["id"],
            "success":true,
            "command":"get_message",
            "data":{"id":"m-final","content":"world","offset":12,"contentLength":17,"hasMoreContent":false}
        })
        .to_string();
        quecto_line_io::write_frame(
            &mut write_half,
            reply.as_bytes(),
            quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
        )
        .await
        .unwrap();
    });

    let mut parent = SubagentEntry::new(sock_path, 0);
    parent.persisted_liveness = crate::domain::session::SubagentLiveness::Live;
    let mut child = SubagentEntry::new(PathBuf::new(), 0);
    child.parent_id = Some("parent".to_string());
    child.persisted_liveness = crate::domain::session::SubagentLiveness::Live;
    let mut grandchild = SubagentEntry::new(PathBuf::new(), 0);
    grandchild.parent_id = Some("child".to_string());
    grandchild.persisted_liveness = crate::domain::session::SubagentLiveness::Live;
    registry
        .lock()
        .unwrap()
        .insert("parent".to_string(), parent);
    registry.lock().unwrap().insert("child".to_string(), child);
    registry
        .lock()
        .unwrap()
        .insert("grandchild".to_string(), grandchild);

    let tool = AgentCmdTool::new(registry);
    let result = tool
        .execute(r#"{"agent_id":"grandchild","command":"get_message","messageId":"m-final","offset":12,"limit":5}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let response: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(response["data"]["content"], "world");
}
