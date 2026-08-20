use crate::domain::tool::{Tool, ToolResult};
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::agent_cmd_report::bounded_report_messages;
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
use std::io::Write;
use std::path::PathBuf;

impl AgentCmdTool {
    fn shape_default_get_messages_report(&self, agent_id: &str, response: &str) -> String {
        let (content, receipt) =
            self.shape_default_get_messages_report_with_metadata(agent_id, response);
        let Some(receipt) = receipt else {
            return content;
        };
        let Ok(mut envelope) = serde_json::from_str::<serde_json::Value>(&content) else {
            return content;
        };
        if let Some(data) = envelope.get_mut("data") {
            data["deliveryReceipt"] = serde_json::json!(receipt);
        }
        envelope.to_string()
    }
}

fn empty_tool() -> AgentCmdTool {
    AgentCmdTool::new(new_registry())
}

fn json_response(messages: serde_json::Value) -> String {
    serde_json::json!({"success": true, "data": {"messages": messages}}).to_string()
}

#[tokio::test]
async fn default_get_messages_backfills_gap_before_shaping() {
    let registry = new_registry();
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("child.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let line = crate::infrastructure::test_support::read_framed_command(&stream).unwrap();
        assert!(line.contains("before-cursor"));
        let id = serde_json::from_str::<serde_json::Value>(&line).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        writeln!(
            stream,
            r#"{{"type":"response","id":"{id}","success":true,"data":{{"messages":[{{"role":"assistant","content":"middle","ordinal":11}}]}}}}"#
        )
        .unwrap();
    });
    let mut entry = SubagentEntry::new(sock_path.clone(), 0);
    entry.delivered_message_ordinal = Some(10);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry);
    let first =
        serde_json::json!({"success": true, "data": {"before":"before-cursor", "messages": [
            {"role":"assistant","content":"tail","ordinal":12}
        ]}})
        .to_string();
    let expanded = tool
        .expand_default_get_messages_response(&sock_path, None, &first, "w1")
        .await;
    let shaped = tool.shape_default_get_messages_report("w1", &expanded);
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    let got: Vec<_> = parsed["data"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap())
        .collect();
    assert_eq!(got, vec!["middle", "tail"]);
}

#[tokio::test]
async fn default_get_messages_backfill_failure_is_marked_incomplete() {
    let registry = new_registry();
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("child.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    let mut entry = SubagentEntry::new(sock_path.clone(), 0);
    entry.delivered_message_ordinal = Some(10);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    let first = serde_json::json!({"success": true, "data": {"messages": [
        {"role":"assistant","content":"tail","ordinal":12}
    ]}})
    .to_string();
    let expanded = tool
        .expand_default_get_messages_response(&sock_path, None, &first, "w1")
        .await;
    let shaped = tool.shape_default_get_messages_report("w1", &expanded);
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    assert_eq!(parsed["data"]["messages"][0]["content"], "tail");
    assert_eq!(parsed["data"]["reportIncomplete"], true);
    assert_eq!(registry.lock().unwrap()["w1"].pending_message_ordinal, None);
}

#[test]
fn default_get_messages_first_call_latest_substantive_assistant_then_ack_advances() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "w1".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0),
    );
    let tool = AgentCmdTool::new(registry.clone());
    let response = json_response(serde_json::json!([
        {"role":"user","content":"u","ordinal":1},
        {"role":"assistant","content":"a1","ordinal":2},
        {"role":"tool","content":"t","ordinal":3},
        {"role":"assistant","content":"a2","ordinal":4},
        {"role":"assistant","content":"","toolCalls":[{"name":"next"}],"ordinal":5}
    ]));
    let shaped = tool.shape_default_get_messages_report("w1", &response);
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    assert_eq!(parsed["data"]["messages"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["data"]["messages"][0]["content"], "a2");
    assert!(
        registry.lock().unwrap()["w1"]
            .delivered_message_ordinal
            .is_none()
    );
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content: shaped,
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        },
    );
    assert_eq!(
        registry.lock().unwrap()["w1"].delivered_message_ordinal,
        Some(4)
    );
}

#[test]
fn first_contact_does_not_commit_omitted_later_non_assistant_ordinals() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "w1".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0),
    );
    let tool = AgentCmdTool::new(registry.clone());
    let shaped = tool.shape_default_get_messages_report(
        "w1",
        &json_response(serde_json::json!([
            {"role":"user","content":"u","ordinal":1},
            {"role":"assistant","content":"a","ordinal":2},
            {"role":"tool","content":"stale tail","ordinal":3}
        ])),
    );
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content: shaped,
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        },
    );
    assert_eq!(
        registry.lock().unwrap()["w1"].delivered_message_ordinal,
        Some(2)
    );
}

#[test]
fn stale_lower_ordinals_do_not_reset_cursor_or_report_duplicate_assistant() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(100);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    let shaped = tool.shape_default_get_messages_report(
        "w1",
        &json_response(serde_json::json!([
            {"role":"assistant","content":"new after prune","ordinal":25}
        ])),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    assert_eq!(parsed["data"], serde_json::json!({"unchanged": true}));
    assert_eq!(registry.lock().unwrap()["w1"].pending_message_ordinal, None);
}

#[test]
fn bounded_report_stops_when_truncated_message_still_cannot_fit() {
    let older = serde_json::json!({
        "id":"00000000-0000-0000-0000-000000000001",
        "role":"assistant",
        "content":"x".repeat(18_000),
        "ordinal":1
    });
    let newer = serde_json::json!({
        "id":"00000000-0000-0000-0000-000000000002",
        "role":"assistant",
        "content":"y".repeat(18_000),
        "ordinal":2
    });
    let report = bounded_report_messages(vec![older, newer], 2);
    assert!(report.has_more_messages);
    assert!(report.messages.len() <= 1);
}

#[test]
fn default_get_messages_later_call_returns_all_roles_after_cursor() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(4);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry);
    let response = json_response(serde_json::json!([
        {"role":"assistant","content":"old","ordinal":4},
        {"role":"user","content":"u2","ordinal":5},
        {"role":"tool","content":"t2","ordinal":6},
        {"role":"assistant","content":"a3","ordinal":7}
    ]));
    let shaped = tool.shape_default_get_messages_report("w1", &response);
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    let got = parsed["data"]["messages"].as_array().unwrap();
    assert_eq!(
        got.iter()
            .map(|m| m["content"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["u2", "t2", "a3"]
    );
}

#[test]
fn clear_history_delivery_resets_default_get_messages_cursor() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(50);
    entry.pending_message_ordinal = Some(60);
    entry
        .pending_message_reports
        .push_back(crate::domain::session::PendingMessageReport {
            receipt: "stale".into(),
            response: "stale-response".into(),
            ordinal: 60,
        });
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"clear_history"}"#,
        &ToolResult {
            content: r#"{"success":true}"#.into(),
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        },
    );
    let entry = &registry.lock().unwrap()["w1"];
    assert_eq!(entry.delivered_message_ordinal, None);
    assert_eq!(entry.pending_message_ordinal, None);
    assert!(entry.pending_message_reports.is_empty());
}

#[test]
fn failed_clear_history_delivery_keeps_default_get_messages_cursor() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(50);
    entry.pending_message_ordinal = Some(60);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"clear_history"}"#,
        &ToolResult {
            content: r#"{"success":false,"error":"busy"}"#.into(),
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        },
    );
    let entry = &registry.lock().unwrap()["w1"];
    assert_eq!(entry.delivered_message_ordinal, Some(50));
    assert_eq!(entry.pending_message_ordinal, Some(60));
}

#[test]
fn malformed_clear_history_delivery_keeps_default_get_messages_cursor() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(50);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"clear_history"}"#,
        &ToolResult {
            content: "ok".into(),
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        },
    );
    assert_eq!(
        registry.lock().unwrap()["w1"].delivered_message_ordinal,
        Some(50)
    );
}

#[test]
fn default_get_messages_unchanged_marker_is_exact_data_shape() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(2);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry);
    let shaped = tool.shape_default_get_messages_report(
        "w1",
        &json_response(serde_json::json!([
            {"role":"assistant","content":"old","ordinal":2}
        ])),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    assert_eq!(parsed["data"], serde_json::json!({"unchanged": true}));
}

#[test]
fn completed_final_response_survives_intermediate_tool_output_crowding() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(1);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    let shaped = tool.shape_default_get_messages_report(
        "w1",
        &json_response(serde_json::json!([
            {"role":"user","content":"work","ordinal":1},
            {"role":"assistant","content":"","thinking":[{"kind":"text","text":"planning"}],"toolCalls":[{"name":"search"}],"ordinal":2},
            {"role":"tool","content":"x".repeat(10_000),"ordinal":3},
            {"role":"assistant","content":"","thinking":[{"kind":"text","text":"revise"}],"toolCalls":[{"name":"read"}],"ordinal":4},
        {"role":"tool","content":"error: missing file","isError":true,"ordinal":5},
        {"role":"assistant","content":"","thinking":[{"kind":"text","text":"try alternate"}],"toolCalls":[{"name":"bash"}],"ordinal":6},
        {"role":"tool","content":"x".repeat(10_000),"ordinal":7},
        {"id":"00000000-0000-0000-0000-000000000008","role":"assistant","content":"the completed final response ".repeat(300),"ordinal":8}
        ])),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    let messages = parsed["data"]["messages"].as_array().unwrap();
    assert!(messages.iter().any(|message| {
        message["ordinal"] == 8
            && message["content"]
                .as_str()
                .is_some_and(|content| !content.is_empty())
    }));
    assert_eq!(parsed["data"]["messageContentTruncated"], true);
    assert!(parsed["data"]["hasMoreMessages"].is_boolean());
    let recovery = &messages
        .iter()
        .find(|message| message["ordinal"] == 8)
        .unwrap()["contentRecovery"];
    assert_eq!(recovery["command"], "get_message");
    assert_eq!(
        recovery["messageId"],
        "00000000-0000-0000-0000-000000000008"
    );
    assert!(recovery["offset"].as_u64().is_some_and(|offset| offset > 0));
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content: shaped,
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        },
    );
    assert_eq!(
        registry.lock().unwrap()["w1"].delivered_message_ordinal,
        Some(8)
    );
    let repeat = tool.shape_default_get_messages_report(
        "w1",
        &json_response(serde_json::json!([
            {"id":"00000000-0000-0000-0000-000000000008","role":"assistant","content":"the completed final response ","ordinal":8}
        ])),
    );
    let repeat: serde_json::Value = serde_json::from_str(&repeat).unwrap();
    assert_eq!(repeat["data"], serde_json::json!({"unchanged": true}));
}

#[test]
fn default_get_messages_bounds_large_delta_and_prioritizes_final_tail() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(1);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    let shaped = tool.shape_default_get_messages_report(
        "w1",
        &json_response(serde_json::json!([
            {"role":"assistant","content":"old","ordinal":1},
            {"id":"00000000-0000-0000-0000-000000000002","role":"assistant","content":"x".repeat(5000),"ordinal":2},
            {"role":"assistant","content":"tail","ordinal":3}
        ])),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    assert_eq!(parsed["data"]["truncated"], true);
    assert_eq!(parsed["data"]["messages"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["data"]["messages"][1]["content"], "tail");
    assert_eq!(
        registry.lock().unwrap()["w1"].pending_message_ordinal,
        Some(3)
    );
}

#[test]
fn get_messages_rejects_malformed_paging_arguments() {
    let tool = empty_tool();
    for args in [
        r#"{"agent_id":"w1","command":"get_messages","count":-1}"#,
        r#"{"agent_id":"w1","command":"get_messages","count":1.5}"#,
        r#"{"agent_id":"w1","command":"get_messages","count":"1"}"#,
        r#"{"agent_id":"w1","command":"get_messages","count":{}}"#,
        r#"{"agent_id":"w1","command":"get_messages","before":123}"#,
        r#"{"agent_id":"w1","command":"get_messages","before":{}}"#,
    ] {
        assert!(tool.parse_and_build(args).is_err(), "{args} should fail");
    }
}

#[test]
fn command_builder_preserves_control_and_model_command_contracts() {
    let tool = empty_tool();
    let cases = [
        (
            r#"{"agent_id":"w1","command":"prompt","message":"do it"}"#,
            serde_json::json!({"type":"prompt","message":"do it","ack":"accept"}),
        ),
        (
            r#"{"agent_id":"w1","command":"steer","message":"turn left"}"#,
            serde_json::json!({"type":"prompt","message":"turn left","streamingBehavior":"steer","ack":"accept"}),
        ),
        (
            r#"{"agent_id":"w1","command":"follow_up","message":"more"}"#,
            serde_json::json!({"type":"follow_up","message":"more","ack":"accept"}),
        ),
        (
            r#"{"agent_id":"w1","command":"set_model","model":"provider/model"}"#,
            serde_json::json!({"type":"set_model","model":"provider/model","ack":"accept"}),
        ),
        (
            r#"{"agent_id":"w1","command":"set_model","provider":"provider","model_id":"model"}"#,
            serde_json::json!({"type":"set_model","provider":"provider","modelId":"model","ack":"accept"}),
        ),
    ];

    for (args, expected) in cases {
        let (_, command, _) = tool.parse_and_build(args).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&command).unwrap(),
            expected
        );
    }
}

#[test]
fn command_builder_reports_missing_or_invalid_model_selection() {
    let tool = empty_tool();
    let missing = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"set_model"}"#)
        .unwrap_err();
    assert!(missing.contains("requires model, or provider + model_id"));

    let incomplete = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"set_model","provider":"provider"}"#)
        .unwrap_err();
    assert!(incomplete.contains("provider requires model_id"));
}

#[test]
fn get_messages_null_paging_arguments_select_plain_report_mode() {
    let tool = empty_tool();
    let (_, cmd, _) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"get_messages","count":null,"before":null}"#)
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&cmd).unwrap(),
        serde_json::json!({"type":"get_messages"})
    );
}

#[test]
fn first_contact_without_assistant_returns_exact_unchanged() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "w1".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0),
    );
    let tool = AgentCmdTool::new(registry);
    let shaped = tool.shape_default_get_messages_report(
        "w1",
        &json_response(serde_json::json!([
            {"role":"user","content":"u","ordinal":1},
            {"role":"tool","content":"t","ordinal":2}
        ])),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    assert_eq!(parsed["data"], serde_json::json!({"unchanged": true}));
}

#[test]
fn default_get_messages_truncates_multibyte_content_safely() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(1);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry);
    let shaped = tool.shape_default_get_messages_report(
        "w1",
        &json_response(serde_json::json!([
            {"role":"assistant","content":"old","ordinal":1},
            {"id":"00000000-0000-0000-0000-000000000002","role":"assistant","content":"é".repeat(2000),"ordinal":2}
        ])),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    assert_eq!(parsed["data"]["truncated"], true);
    assert!(
        parsed["data"]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .is_char_boundary(
                parsed["data"]["messages"][0]["content"]
                    .as_str()
                    .unwrap()
                    .len()
            )
    );
}

#[test]
fn unchanged_default_get_messages_keeps_pending_without_commit() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(1);
    entry.pending_message_ordinal = Some(2);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    let shaped = tool.shape_default_get_messages_report(
        "w1",
        &json_response(serde_json::json!([
            {"role":"assistant","content":"old","ordinal":1}
        ])),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    assert_eq!(parsed["data"], serde_json::json!({"unchanged": true}));
    assert_eq!(
        registry.lock().unwrap()["w1"].pending_message_ordinal,
        Some(2)
    );
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content: shaped,
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        },
    );
    assert_eq!(
        registry.lock().unwrap()["w1"].delivered_message_ordinal,
        Some(1)
    );
}

#[test]
fn failed_default_get_messages_delivery_keeps_pending_without_commit() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(1);
    entry.pending_message_ordinal = Some(10);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content: r#"{"success":false,"error":"busy"}"#.into(),
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        },
    );
    let entry = &registry.lock().unwrap()["w1"];
    assert_eq!(entry.delivered_message_ordinal, Some(1));
    assert_eq!(entry.pending_message_ordinal, Some(10));
}

#[test]
fn incomplete_default_backfill_returns_uncommitted_marker() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(10);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    let shaped = tool.shape_default_get_messages_report(
        "w1",
        &serde_json::json!({
            "success": true,
            "data": {
                "messages": [{"role":"assistant","content":"later","ordinal":37}],
                "reportIncomplete": true
            }
        })
        .to_string(),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    assert_eq!(parsed["data"]["messages"][0]["content"], "later");
    assert_eq!(parsed["data"]["reportIncomplete"], true);
    assert_eq!(registry.lock().unwrap()["w1"].pending_message_ordinal, None);
}

#[test]
fn incomplete_default_get_messages_delivery_keeps_pending_without_commit() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(1);
    entry.pending_message_ordinal = Some(10);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content: r#"{"success":true,"data":{"unchanged":true,"reportIncomplete":true}}"#.into(),
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        },
    );
    let entry = &registry.lock().unwrap()["w1"];
    assert_eq!(entry.delivered_message_ordinal, Some(1));
    assert_eq!(entry.pending_message_ordinal, Some(10));
}

#[test]
fn first_contact_backfill_not_needed_when_newest_page_has_assistant() {
    let messages = vec![serde_json::json!({"role":"assistant","content":"latest","ordinal":9})];
    assert!(!super::needs_default_report_backfill(&messages, 0));
}

#[test]
fn later_delta_backfill_needed_when_gap_before_newest_page() {
    let messages = vec![serde_json::json!({"role":"assistant","content":"later","ordinal":37})];
    assert!(super::needs_default_report_backfill(&messages, 10));
}

#[test]
fn default_get_messages_strips_unbounded_payloads_to_fit_envelope_budget() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(1);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry);
    let shaped = tool.shape_default_get_messages_report(
        "w1",
        &json_response(serde_json::json!([
            {"role":"assistant","content":"old","ordinal":1},
            {"role":"assistant","content":"é".repeat(2000),"ordinal":2,
             "toolCalls":[{"arguments":"x".repeat(20_000)}],
             "tool_calls":[{"arguments":"x".repeat(20_000)}],
             "imageBlocks":[{"data":"y".repeat(20_000)}],
             "image_blocks":[{"data":"y".repeat(20_000)}]}
        ])),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    assert!(
        serde_json::to_vec(&parsed["data"]).unwrap().len()
            <= crate::infrastructure::tools::agent_cmd_report::REPORT_BUDGET_BYTES
    );
    for key in ["toolCalls", "tool_calls", "imageBlocks", "image_blocks"] {
        assert!(parsed["data"]["messages"][0].get(key).is_none(), "{key}");
    }
}

#[test]
fn incomplete_backfill_returns_bounded_progress_without_advancing_cursor() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(10);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    let shaped = tool.shape_default_get_messages_report(
        "w1",
        &serde_json::json!({"success": true, "data": {"messages": [
            {"role":"assistant","content":"progress","ordinal":37}
        ], "reportIncomplete": true}})
        .to_string(),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    assert_eq!(parsed["data"]["messages"][0]["content"], "progress");
    assert_eq!(parsed["data"]["reportIncomplete"], true);
    assert_eq!(registry.lock().unwrap()["w1"].pending_message_ordinal, None);
}
