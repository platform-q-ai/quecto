//! An ended child's report (#2192 review): the default `get_messages` read
//! and `get_report`, answered from its persisted transcript under the same
//! contract a live child's are (#2226, #2114). The default read is planned
//! by the one default-report rule — the delivered watermark, the report
//! chosen by turn origin, a receipt the delivery acknowledges — so a report
//! followed by many messages is still delivered, and one delivered is not
//! replayed. The final report is carried whole up to the final-report
//! budget, cut beyond it with the same notice.
use crate::application::subagents::use_cases::{EndedTranscriptError, InspectEndedChild};
use crate::domain::message::Message;
use crate::domain::tool::ToolResult;

use super::super::agent_cmd_report::{
    FINAL_REPORT_BUDGET_BYTES, bounded_report_messages, needs_default_report_backfill,
    report_position, shape_default_report_of,
};
use super::super::subagent_registry::SubagentRegistry;
use super::{EndedRow, error, ok};

/// Whether `arguments` ask `get_messages` for the default report: no
/// `count`, no `before`.
pub fn is_default_read(command: &str, arguments: &serde_json::Value) -> bool {
    command == "get_messages"
        && arguments
            .get("count")
            .is_none_or(serde_json::Value::is_null)
        && arguments
            .get("before")
            .is_none_or(serde_json::Value::is_null)
}

/// `message` as a child's `get_messages` page carries it, its content whole
/// (the report budget cuts it, never this).
fn wire_message(message: &Message) -> serde_json::Value {
    let mut value = super::project(message, usize::MAX, usize::MAX);
    if let Some(origin) = crate::infrastructure::turn_origin_names::origin_name(message.turn_origin)
    {
        value["turnOrigin"] = serde_json::json!(origin);
    }
    value
}

/// The newest window of the child's transcript as wire messages, and
/// whether older messages exist beyond it.
async fn window(
    inspection: &InspectEndedChild,
    row: &EndedRow,
) -> Result<(Vec<serde_json::Value>, bool), EndedTranscriptError> {
    let page = inspection
        .transcript(
            &row.uuid,
            row.origin,
            InspectEndedChild::MAX_PAGE_MESSAGES,
            None,
        )
        .await?;
    let messages = page.messages.iter().map(wire_message).collect();
    Ok((messages, page.has_more_before || page.older_omitted))
}

fn unreadable(row: &EndedRow, reason: &str, why: &EndedTranscriptError) -> ToolResult {
    error(format!(
        "subagent '{}' {reason}; its transcript isn't readable from here because {why}",
        row.label
    ))
}

/// Mark `content`'s data as an ended child's: its end, and where it came from.
fn as_ended(content: &str, row: &EndedRow, reason: &str) -> String {
    let Ok(mut envelope) = serde_json::from_str::<serde_json::Value>(content) else {
        return content.to_string();
    };
    if let Some(data) = envelope.get_mut("data").filter(|data| data.is_object()) {
        data["ended"] = serde_json::json!(true);
        data["endReason"] = serde_json::json!(format!("subagent '{}' {reason}", row.label));
        data["source"] = serde_json::json!("persisted transcript");
    }
    envelope.to_string()
}

/// The default `get_messages` of an ended child, for its row in
/// `registry`: its unread report, planned against that row's watermark and
/// left pending for the delivery to acknowledge.
pub async fn default_read(
    registry: &SubagentRegistry,
    inspection: &InspectEndedChild,
    row: &EndedRow,
    reason: &str,
) -> ToolResult {
    let (messages, has_older) = match window(inspection, row).await {
        Ok(window) => window,
        Err(why) => return unreadable(row, reason, &why),
    };
    let delivered = registry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&row.key)
        .and_then(|entry| entry.delivered_message_ordinal)
        .unwrap_or(0);
    // The window is as far back as an ended child's read goes: a report
    // older than it is owed, and the read says it is incomplete.
    let incomplete = needs_default_report_backfill(&messages, delivered, has_older);
    let mut data = serde_json::json!({"messages": messages, "hasMoreBefore": has_older});
    if incomplete {
        data["reportIncomplete"] = serde_json::json!(true);
    }
    let response = ok("get_messages", data).content;
    let (content, receipt) = {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        shape_default_report_of(&mut entries, &row.key, &response)
    };
    ToolResult {
        content: as_ended(&content, row, reason),
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: receipt,
    }
}

/// `get_report` of an ended child: its final report — the reply its
/// window's report rule picks — whole up to the final-report budget.
pub async fn final_report(
    inspection: &InspectEndedChild,
    row: &EndedRow,
    reason: &str,
) -> ToolResult {
    let (messages, _) = match window(inspection, row).await {
        Ok(window) => window,
        Err(why) => return unreadable(row, reason, &why),
    };
    let data = match report_position(&messages) {
        Some(index) => {
            let bounded = bounded_report_messages(vec![messages[index].clone()], 0);
            let mut report = bounded.messages.into_iter().next().unwrap_or_default();
            report["contentTruncated"] = serde_json::json!(bounded.message_content_truncated);
            assert!(
                serde_json::to_vec(&report).map_or(0, |v| v.len())
                    <= FINAL_REPORT_BUDGET_BYTES + 1024,
                "the report stays within its budget"
            );
            serde_json::json!({"report": report, "reportFound": true})
        }
        None => serde_json::json!({"report": null, "reportFound": false}),
    };
    let content = ok("get_report", data).content;
    ok_with(as_ended(&content, row, reason))
}

fn ok_with(content: String) -> ToolResult {
    ToolResult {
        content,
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

#[cfg(test)]
#[path = "agent_cmd_ended_report_tests.rs"]
mod tests;
