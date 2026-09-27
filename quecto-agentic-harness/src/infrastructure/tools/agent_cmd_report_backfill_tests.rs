//! When the default report pages older history in (#2218): only when the
//! page says older history exists, and measured from the page's smallest
//! ordinal, so a whole transcript whose recall notice is numbered after
//! the task (`[2, 1, 3]`), or a failed first turn, is complete.
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
use std::path::PathBuf;

/// #2218: the gap is measured from the page's smallest ordinal, not its
/// first message: a recall notice re-inserted at the head after the task
/// was saved carries a higher ordinal than the task behind it.
#[test]
fn the_gap_is_measured_from_the_smallest_ordinal_of_the_page() {
    let messages = vec![
        serde_json::json!({"role":"system","content":"[Session memory]","ordinal":12}),
        serde_json::json!({"role":"user","content":"task","ordinal":11}),
        serde_json::json!({"role":"assistant","content":"report","ordinal":13}),
    ];
    assert!(!super::super::needs_default_report_backfill(
        &messages, 10, true
    ));
}

/// #2218: a failed first turn leaves `[system 2, user 1]` and nothing
/// older; the report is complete (there is nothing to report yet).
#[test]
fn a_page_with_no_older_history_is_complete() {
    let messages = vec![
        serde_json::json!({"role":"system","content":"[Session memory]","ordinal":2}),
        serde_json::json!({"role":"user","content":"task","ordinal":1}),
    ];
    assert!(!super::super::needs_default_report_backfill(
        &messages, 0, false
    ));
    let later = vec![serde_json::json!({"role":"assistant","content":"later","ordinal":37})];
    assert!(!super::super::needs_default_report_backfill(
        &later, 10, false
    ));
}

/// #2218: a page that does not say older history exists is not paged back
/// from, however far its ordinals lie above the watermark.
#[tokio::test]
async fn a_page_without_older_history_is_never_backfilled_or_incomplete() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(PathBuf::from("/nonexistent/child.sock"), 0);
    entry.delivered_message_ordinal = Some(10);
    registry.lock().unwrap().insert("w1".to_string(), entry);
    let tool = AgentCmdTool::new(registry);
    let first = serde_json::json!({"success": true, "data": {"messages": [
        {"role":"assistant","content":"tail","ordinal":37}
    ]}})
    .to_string();
    let expanded = tool
        .expand_default_get_messages_response(
            std::path::Path::new("/nonexistent/child.sock"),
            None,
            &first,
            "w1",
        )
        .await;
    let parsed: serde_json::Value = serde_json::from_str(&expanded).unwrap();
    assert_eq!(parsed["data"]["reportIncomplete"], serde_json::Value::Null);
    assert_eq!(parsed["data"]["messages"][0]["content"], "tail");
}
