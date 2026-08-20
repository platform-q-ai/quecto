use crate::domain::tool::{Tool, ToolResult};
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
use std::collections::VecDeque;
use std::path::PathBuf;

#[test]
fn report_planning_is_pure_and_returns_pending_transition() {
    let response = json_response(serde_json::json!([
        {"role":"assistant","content":"old","ordinal":1},
        {"role":"assistant","content":"new","ordinal":2}
    ]));
    let plan = crate::infrastructure::tools::agent_cmd_report::plan_default_report(&response, 1);
    let pending = plan.pending.expect("new unread report should be pending");
    assert_eq!(pending.ordinal, 2);
    assert!(!pending.receipt.is_empty());
    assert_eq!(pending.response, plan.content);
}

#[test]
fn delivery_planning_is_pure_and_returns_acknowledgement_transition() {
    let pending = VecDeque::from([crate::domain::session::PendingMessageReport {
        receipt: "receipt-1".into(),
        response: "content".into(),
        ordinal: 2,
    }]);
    let decision = crate::infrastructure::tools::agent_cmd_report::plan_delivery(
        Some("get_messages"),
        false,
        false,
        r#"{"success":true,"data":{"messages":[]}}"#,
        Some("receipt-1"),
        &pending,
    );
    assert_eq!(
        decision,
        crate::infrastructure::tools::agent_cmd_report::DeliveryDecision::Acknowledge(0)
    );
}

fn json_response(messages: serde_json::Value) -> String {
    serde_json::json!({"success": true, "data": {"messages": messages}}).to_string()
}

#[test]
fn delivery_receipts_prevent_byte_identical_response_collision() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry
        .pending_message_reports
        .push_back(crate::domain::session::PendingMessageReport {
            receipt: "r1".into(),
            response: serde_json::json!({"success":true,"data":{"messages":["same"]}}).to_string(),
            ordinal: 5,
        });
    entry
        .pending_message_reports
        .push_back(crate::domain::session::PendingMessageReport {
            receipt: "r2".into(),
            response: serde_json::json!({"success":true,"data":{"messages":["same"]}}).to_string(),
            ordinal: 9,
        });
    entry.pending_message_ordinal = Some(9);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content:
                serde_json::json!({"success":true,"data":{"deliveryReceipt":"r2","messages":[]}})
                    .to_string(),
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        },
    );
    let entry = &registry.lock().unwrap()["w1"];
    assert_eq!(entry.delivered_message_ordinal, Some(9));
    assert_eq!(entry.pending_message_reports.len(), 1);
    assert_eq!(entry.pending_message_reports[0].receipt, "r1");
}

#[test]
fn missing_or_unknown_delivery_receipt_does_not_ack_new_pending_report() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry
        .pending_message_reports
        .push_back(crate::domain::session::PendingMessageReport {
            receipt: "real".into(),
            response: serde_json::json!({"success":true,"data":{"messages":["same"]}}).to_string(),
            ordinal: 5,
        });
    entry.pending_message_ordinal = Some(5);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    for content in [
        serde_json::json!({"success":true,"data":{"messages":[]}}).to_string(),
        serde_json::json!({"success":true,"data":{"deliveryReceipt":"unknown","messages":[]}})
            .to_string(),
    ] {
        tool.result_delivered(
            r#"{"agent_id":"w1","command":"get_messages"}"#,
            &ToolResult {
                content,
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            },
        );
    }
    let entry = &registry.lock().unwrap()["w1"];
    assert_eq!(entry.delivered_message_ordinal, None);
    assert_eq!(entry.pending_message_ordinal, Some(5));
    assert_eq!(entry.pending_message_reports.len(), 1);
}

#[test]
fn explicit_paging_result_delivery_is_cursor_neutral_even_with_new_ordinals() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(4);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    let result = ToolResult {
        content: json_response(
            serde_json::json!([{"role":"assistant","content":"new","ordinal":99}]),
        ),
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: None,
    };
    for args in [
        r#"{"agent_id":"w1","command":"get_messages","count":1}"#,
        r#"{"agent_id":"w1","command":"get_messages","before":"cursor"}"#,
        r#"{"agent_id":"w1","command":"get_messages","count":1,"before":"cursor"}"#,
    ] {
        tool.result_delivered(args, &result);
    }
    let entry = &registry.lock().unwrap()["w1"];
    assert_eq!(entry.delivered_message_ordinal, Some(4));
    assert_eq!(entry.pending_message_ordinal, None);
    assert!(entry.pending_message_reports.is_empty());
}

#[test]
fn legacy_pending_report_deserializes_without_receipt() {
    let pending: crate::domain::session::PendingMessageReport =
        serde_json::from_str(r#"{"response":"legacy-response","ordinal":7}"#).unwrap();
    assert_eq!(pending.receipt, "");
    assert_eq!(pending.response, "legacy-response");
    assert_eq!(pending.ordinal, 7);
}

#[test]
fn delivery_receipts_are_opaque_unique_tokens() {
    let first = crate::infrastructure::tools::agent_cmd_report::mint_default_report_receipt();
    let second = crate::infrastructure::tools::agent_cmd_report::mint_default_report_receipt();
    assert!(first.starts_with("agent-cmd-report-"));
    assert!(second.starts_with("agent-cmd-report-"));
    assert_ne!(first, second);
}

#[test]
fn duplicate_legacy_pending_responses_are_ambiguous_and_do_not_ack() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry
        .pending_message_reports
        .push_back(crate::domain::session::PendingMessageReport {
            receipt: "".into(),
            response: serde_json::json!({"success":true,"data":{"messages":["same"]}}).to_string(),
            ordinal: 5,
        });
    entry
        .pending_message_reports
        .push_back(crate::domain::session::PendingMessageReport {
            receipt: "".into(),
            response: serde_json::json!({"success":true,"data":{"messages":["same"]}}).to_string(),
            ordinal: 9,
        });
    entry.pending_message_ordinal = Some(9);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content: serde_json::json!({"success":true,"data":{"messages":["same"]}}).to_string(),
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        },
    );
    let entry = &registry.lock().unwrap()["w1"];
    assert_eq!(entry.delivered_message_ordinal, None);
    assert_eq!(entry.pending_message_ordinal, Some(9));
    assert_eq!(entry.pending_message_reports.len(), 2);
}

#[test]
fn single_legacy_pending_response_acknowledges_by_response_equality() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry
        .pending_message_reports
        .push_back(crate::domain::session::PendingMessageReport {
            receipt: "".into(),
            response: serde_json::json!({"success":true,"data":{"messages":["legacy"]}})
                .to_string(),
            ordinal: 7,
        });
    entry.pending_message_ordinal = Some(7);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content: serde_json::json!({"success":true,"data":{"messages":["legacy"]}}).to_string(),
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        },
    );
    let entry = &registry.lock().unwrap()["w1"];
    assert_eq!(entry.delivered_message_ordinal, Some(7));
    assert_eq!(entry.pending_message_ordinal, None);
    assert!(entry.pending_message_reports.is_empty());
}

#[test]
fn delivery_metadata_ack_does_not_expose_receipt_in_content() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry
        .pending_message_reports
        .push_back(crate::domain::session::PendingMessageReport {
            receipt: "secret".into(),
            response: serde_json::json!({"success":true,"data":{"messages":[]}}).to_string(),
            ordinal: 11,
        });
    entry.pending_message_ordinal = Some(11);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());
    let content = serde_json::json!({"success":true,"data":{"messages":[]}}).to_string();
    assert!(!content.contains("deliveryReceipt"));
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content,
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: Some("secret".into()),
        },
    );
    let entry = &registry.lock().unwrap()["w1"];
    assert_eq!(entry.delivered_message_ordinal, Some(11));
    assert!(entry.pending_message_reports.is_empty());
}

#[test]
fn pending_delivery_correlates_to_each_shaped_get_messages_result() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(1);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry.clone());

    let (first, first_receipt) = tool.shape_default_get_messages_report_with_metadata(
        "w1",
        &json_response(serde_json::json!([
            {"role":"assistant","content":"old","ordinal":1},
            {"role":"assistant","content":"first","ordinal":2}
        ])),
    );
    let (second, second_receipt) = tool.shape_default_get_messages_report_with_metadata(
        "w1",
        &json_response(serde_json::json!([
            {"role":"assistant","content":"old","ordinal":1},
            {"role":"assistant","content":"first","ordinal":2},
            {"role":"assistant","content":"second","ordinal":3}
        ])),
    );

    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content: first,
            is_error: false,
            image_blocks: Vec::new(),
            delivery_metadata: first_receipt,
        },
    );
    assert_eq!(
        registry.lock().unwrap()["w1"].delivered_message_ordinal,
        Some(2)
    );
    assert_eq!(
        registry.lock().unwrap()["w1"].pending_message_ordinal,
        Some(3)
    );

    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content: second,
            is_error: false,
            image_blocks: Vec::new(),
            delivery_metadata: second_receipt,
        },
    );
    assert_eq!(
        registry.lock().unwrap()["w1"].delivered_message_ordinal,
        Some(3)
    );
    assert_eq!(registry.lock().unwrap()["w1"].pending_message_ordinal, None);
}
