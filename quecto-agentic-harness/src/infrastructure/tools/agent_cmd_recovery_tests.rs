//! #2114: how the default report treats a long final message — delivered
//! whole within the final-report budget, cut with a plain notice beyond it
//! (never a recovery command), and undelivered when it has no text at all.
use crate::application::tools::ports::Tool;
use crate::domain::tool::ToolResult;
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::agent_cmd_report::{
    FINAL_REPORT_BUDGET_BYTES, FINAL_REPORT_NOTICE, bounded_report_messages,
};
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
use std::path::PathBuf;

fn json_response(messages: serde_json::Value) -> String {
    serde_json::json!({"success": true, "data": {"messages": messages}}).to_string()
}

fn tool_with_delivered(
    ordinal: u64,
) -> (
    AgentCmdTool,
    crate::infrastructure::tools::subagent_registry::SubagentRegistry,
) {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0);
    entry.delivered_message_ordinal = Some(ordinal);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    (AgentCmdTool::new(registry.clone()), registry)
}

fn deliver(tool: &AgentCmdTool, shaped: String, receipt: Option<String>) {
    tool.result_delivered(
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        &ToolResult {
            content: shaped,
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: receipt,
        },
    );
}

#[test]
fn a_final_report_within_the_budget_is_delivered_whole_and_acknowledged() {
    let (tool, registry) = tool_with_delivered(1);
    let full = "final ".repeat(10_000);
    assert!(full.len() < FINAL_REPORT_BUDGET_BYTES);
    let (shaped, receipt) = tool.shape_default_get_messages_report_with_metadata(
        "w1",
        &json_response(serde_json::json!([
            {"role":"assistant","content":"old","ordinal":1},
            {"id":"m2","role":"assistant","content":full,"ordinal":2}
        ])),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    let reported = &parsed["data"]["messages"][0];
    assert_eq!(reported["content"].as_str().unwrap(), full);
    assert_ne!(reported["truncated"], true);
    assert_eq!(parsed["data"]["messageContentTruncated"], false);
    deliver(&tool, shaped, receipt);
    assert_eq!(
        registry.lock().unwrap()["w1"].delivered_message_ordinal,
        Some(2)
    );
}

#[test]
fn a_final_report_beyond_the_budget_is_cut_with_a_notice_and_acknowledged() {
    let (tool, registry) = tool_with_delivered(1);
    let full = "final ".repeat(FINAL_REPORT_BUDGET_BYTES / 6 + 2_000);
    let (shaped, receipt) = tool.shape_default_get_messages_report_with_metadata(
        "w1",
        &json_response(serde_json::json!([
            {"role":"assistant","content":"old","ordinal":1},
            {"id":"m2","role":"assistant","content":full,"ordinal":2}
        ])),
    );
    assert!(!shaped.contains("contentRecovery"), "no recovery command");
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    let reported = &parsed["data"]["messages"][0];
    let kept = reported["content"].as_str().unwrap();
    assert!(full.starts_with(kept) && kept.len() > FINAL_REPORT_BUDGET_BYTES / 2);
    assert!(shaped.len() <= FINAL_REPORT_BUDGET_BYTES + 3_200 + 2_000);
    assert_eq!(reported["truncated"], true);
    assert_eq!(reported["contentLength"], full.len());
    assert_eq!(reported["contentNotice"], FINAL_REPORT_NOTICE);
    deliver(&tool, shaped, receipt);
    assert_eq!(
        registry.lock().unwrap()["w1"].delivered_message_ordinal,
        Some(2)
    );
}

#[test]
fn a_final_message_without_text_is_not_delivered() {
    let (tool, registry) = tool_with_delivered(1);
    let (shaped, receipt) = tool.shape_default_get_messages_report_with_metadata(
        "w1",
        &json_response(serde_json::json!([
            {"role":"assistant","content":"old","ordinal":1},
            {"role":"assistant","content":{"omitted":"x".repeat(80_000)},"ordinal":2}
        ])),
    );
    let parsed: serde_json::Value = serde_json::from_str(&shaped).unwrap();
    assert!(parsed["data"]["messages"].as_array().unwrap().is_empty());
    assert_eq!(parsed["data"]["hasMoreMessages"], true);
    assert_eq!(parsed["data"]["reportIncomplete"], true);
    deliver(&tool, shaped, receipt);
    assert_eq!(
        registry.lock().unwrap()["w1"].delivered_message_ordinal,
        Some(1)
    );
}

/// #2114: a context message cut down to no text at all is never delivered,
/// whatever the space left beside the final report (swept across the edge
/// where only its empty shell would fit).
#[test]
fn a_context_message_cut_to_nothing_is_not_delivered() {
    for pad in 2_900..3_300 {
        let context = serde_json::json!({
            "role": "user", "ordinal": 1, "content": "x".repeat(5_000), "pad": "p".repeat(pad)
        });
        let handoff = serde_json::json!({"role": "assistant", "ordinal": 2, "content": "done"});
        let report = bounded_report_messages(vec![context, handoff], 2);
        for message in &report.messages {
            let empty = message["content"].as_str().is_some_and(str::is_empty);
            assert!(
                !(empty && message["truncated"] == true),
                "pad {pad}: delivered an emptied message {message}"
            );
        }
    }
}

#[test]
fn a_context_message_cut_to_fit_says_how_to_read_it_whole() {
    let context = serde_json::json!({"role": "user", "ordinal": 1, "content": "x".repeat(20_000)});
    let handoff = serde_json::json!({"role": "assistant", "ordinal": 2, "content": "done"});
    let report = bounded_report_messages(vec![context, handoff], 2);
    let cut = &report.messages[0];
    assert_eq!(cut["truncated"], true);
    assert_eq!(
        cut["contentNotice"],
        crate::infrastructure::tools::agent_cmd_report::CONTEXT_CUT_NOTICE
    );
}
