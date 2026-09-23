//! The `agent_cmd` default unread report (#1856): the adapter side of the
//! supervisor's plain `get_messages` read. The child's wire response is
//! parsed here, the selection and acknowledgement rules are the domain's
//! (`domain::unread_report`), and the report budget, receipt and envelope
//! shaping stay with this tool.
use crate::domain::session::PendingMessageReport;
use crate::domain::unread_report::{
    ReportedMessage, UnreadSelection, acknowledged_report_index, needs_backfill, select_unread,
};

pub(crate) fn mint_default_report_receipt() -> String {
    format!("agent-cmd-report-{}", uuid::Uuid::new_v4())
}

pub(crate) fn delivery_receipt(response: &serde_json::Value) -> Option<&str> {
    response
        .pointer("/data/deliveryReceipt")
        .and_then(|v| v.as_str())
        .filter(|receipt| !receipt.is_empty())
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DefaultReportPlan {
    pub content: String,
    pub pending: Option<PendingMessageReport>,
}

fn ordinal_of(message: &serde_json::Value) -> Option<u64> {
    message.get("ordinal").and_then(|value| value.as_u64())
}

fn observed(message: &serde_json::Value) -> ReportedMessage {
    ReportedMessage {
        ordinal: ordinal_of(message),
        substantive_assistant: is_substantive_assistant(message),
    }
}

pub(crate) fn plan_default_report(response: &str, delivered: u64) -> DefaultReportPlan {
    let unchanged = |content: String| DefaultReportPlan {
        content,
        pending: None,
    };
    let Ok(mut envelope) = serde_json::from_str::<serde_json::Value>(response) else {
        return unchanged(response.to_string());
    };
    if envelope.get("success").and_then(|v| v.as_bool()) == Some(false) {
        return unchanged(response.to_string());
    }
    let Some(data) = envelope.get_mut("data") else {
        return unchanged(response.to_string());
    };
    let report_incomplete = data.get("reportIncomplete").and_then(|v| v.as_bool()) == Some(true);
    let Some(messages) = data.get_mut("messages").and_then(|v| v.as_array_mut()) else {
        return unchanged(response.to_string());
    };
    let selected = |indices: &[usize]| -> Vec<serde_json::Value> {
        indices
            .iter()
            .map(|&index| messages[index].clone())
            .collect()
    };
    let reported: Vec<ReportedMessage> = messages.iter().map(observed).collect();
    match select_unread(&reported, delivered, report_incomplete) {
        UnreadSelection::PendingPersistence { latest_substantive } => {
            let report = bounded_report_messages(
                selected(&latest_substantive.into_iter().collect::<Vec<_>>()),
                0,
            );
            *data = serde_json::json!({"messages":report.messages, "cursorNeutral":true,
                "ordinalStatus":"pending_persistence", "reportIncomplete":true,
                "messageContentTruncated":report.message_content_truncated});
            unchanged(envelope.to_string())
        }
        UnreadSelection::Incomplete {
            unread,
            max_ordinal,
        } => {
            if !unread.is_empty() {
                let report = bounded_report_messages(selected(&unread), max_ordinal);
                if !report.messages.is_empty() {
                    *data = serde_json::json!({"messages": report.messages, "truncated": true, "hasMoreMessages": report.has_more_messages, "messageContentTruncated": report.message_content_truncated, "reportIncomplete": true});
                    return unchanged(envelope.to_string());
                }
            }
            *data = serde_json::json!({"unchanged": true, "reportIncomplete": true});
            unchanged(envelope.to_string())
        }
        UnreadSelection::Unchanged => {
            *data = serde_json::json!({"unchanged": true});
            unchanged(envelope.to_string())
        }
        UnreadSelection::Unread {
            indices,
            max_ordinal,
        } => {
            let report = bounded_report_messages(selected(&indices), max_ordinal);
            if report.messages.is_empty() {
                *data = if max_ordinal > delivered {
                    serde_json::json!({"messages": [], "truncated": true, "hasMoreMessages": true, "messageContentTruncated": false, "reportIncomplete": true})
                } else {
                    serde_json::json!({"unchanged": true})
                };
                return unchanged(envelope.to_string());
            }
            let ordinal = report
                .messages
                .iter()
                .filter_map(ordinal_of)
                .max()
                .unwrap_or(delivered);
            let truncated = report.has_more_messages || report.message_content_truncated;
            let receipt = mint_default_report_receipt();
            *data = serde_json::json!({"messages": report.messages, "truncated": truncated, "hasMoreMessages": report.has_more_messages, "messageContentTruncated": report.message_content_truncated});
            let content = envelope.to_string();
            DefaultReportPlan {
                content: content.clone(),
                pending: Some(PendingMessageReport {
                    receipt,
                    response: content,
                    ordinal,
                }),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeliveryDecision {
    Ignore,
    Clear,
    Acknowledge(usize),
}

pub(crate) fn plan_delivery(
    command: Option<&str>,
    explicit_page: bool,
    result_is_error: bool,
    content: &str,
    metadata_receipt: Option<&str>,
    pending: &std::collections::VecDeque<PendingMessageReport>,
) -> DeliveryDecision {
    if result_is_error {
        return DeliveryDecision::Ignore;
    }
    if command == Some("clear_history") {
        return if serde_json::from_str::<serde_json::Value>(content)
            .ok()
            .and_then(|v| v.get("success").and_then(|v| v.as_bool()))
            == Some(true)
        {
            DeliveryDecision::Clear
        } else {
            DeliveryDecision::Ignore
        };
    }
    if command != Some("get_messages") || explicit_page {
        return DeliveryDecision::Ignore;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return DeliveryDecision::Ignore;
    };
    if value.get("success").and_then(|v| v.as_bool()) != Some(true)
        || value
            .pointer("/data/reportIncomplete")
            .and_then(|v| v.as_bool())
            == Some(true)
    {
        return DeliveryDecision::Ignore;
    }
    let receipt = metadata_receipt.or_else(|| delivery_receipt(&value));
    acknowledged_report_index(pending, receipt, content)
        .map(DeliveryDecision::Acknowledge)
        .unwrap_or(DeliveryDecision::Ignore)
}

pub(crate) fn needs_default_report_backfill(
    messages: &[serde_json::Value],
    delivered: u64,
) -> bool {
    let holds_assistant = messages
        .iter()
        .any(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"));
    needs_backfill(
        messages.first().and_then(ordinal_of),
        holds_assistant,
        delivered,
    )
}

pub(crate) const REPORT_BUDGET_BYTES: usize = 800 * 4;
/// A finished child's final report is delivered whole up to this size
/// (#2114); beyond it the start is kept with a plain notice.
pub(crate) const FINAL_REPORT_BUDGET_BYTES: usize = 64 * 1024;

pub(crate) struct BoundedReport {
    pub messages: Vec<serde_json::Value>,
    pub has_more_messages: bool,
    pub message_content_truncated: bool,
}

pub(crate) fn bounded_report_messages(
    mut candidates: Vec<serde_json::Value>,
    max_available_ordinal: u64,
) -> BoundedReport {
    for msg in &mut candidates {
        strip_unbounded_payloads(msg);
    }

    // The latest substantive assistant message is the child's handoff: it
    // gets its own budget (#2114) and the rest of the report, context, the
    // ordinary one on top of it.
    let final_idx = candidates.iter().rposition(is_substantive_assistant);
    if let Some(index) = final_idx {
        let final_message = &mut candidates[index];
        if report_envelope_size(&[], Some(final_message)) > FINAL_REPORT_BUDGET_BYTES {
            truncate_message_to_fit(final_message, &[], FINAL_REPORT_BUDGET_BYTES);
            final_message["contentNotice"] = serde_json::json!(FINAL_REPORT_NOTICE);
        }
    }
    let budget = final_idx
        .map(|index| report_envelope_size(&[], Some(&candidates[index])))
        .unwrap_or(0)
        + REPORT_BUDGET_BYTES;

    // Preserve transcript order when it already fits. If it does not, reserve the
    // envelope for the latest substantive assistant handoff before optional context.
    let all_fit = report_envelope_size(&candidates, None) <= budget;
    let mut ordered = if all_fit {
        candidates
    } else if let Some(final_idx) = final_idx {
        let final_message = candidates.remove(final_idx);
        let mut prioritized = vec![final_message];
        prioritized.extend(candidates.into_iter().rev());
        prioritized
    } else {
        // During a long tool-only turn, report current progress rather than
        // forcing the supervisor to page through the oldest unread tools.
        candidates.into_iter().rev().collect()
    };

    let candidate_count = ordered.len();
    let mut selected = Vec::new();
    for mut msg in ordered.drain(..) {
        if report_envelope_size(&selected, Some(&msg)) > budget {
            truncate_message_to_fit(&mut msg, &selected, budget);
        }
        if is_emptied_by_truncation(&msg) {
            break;
        }
        if report_envelope_size(&selected, Some(&msg)) <= budget {
            selected.push(msg);
        } else {
            break;
        }
    }
    if !all_fit {
        selected.sort_by_key(|m| m.get("ordinal").and_then(|v| v.as_u64()).unwrap_or(0));
    }
    let committed = selected
        .iter()
        .filter_map(|m| m.get("ordinal").and_then(|v| v.as_u64()))
        .max()
        .unwrap_or(0);
    let message_content_truncated = selected
        .iter()
        .any(|m| m.get("truncated").and_then(|v| v.as_bool()) == Some(true));
    BoundedReport {
        has_more_messages: selected.len() < candidate_count || committed < max_available_ordinal,
        message_content_truncated,
        messages: selected,
    }
}

/// Shown on a final report cut at [`FINAL_REPORT_BUDGET_BYTES`].
pub(crate) const FINAL_REPORT_NOTICE: &str = "This report is longer than the report budget and was cut; its start is shown. Call agent_cmd get_report with export_raw true to write the child's full retained history to an artifact you can read.";

pub(crate) fn is_substantive_assistant(msg: &serde_json::Value) -> bool {
    msg.get("role").and_then(|v| v.as_str()) == Some("assistant")
        && msg
            .get("content")
            .and_then(|v| v.as_str())
            .is_some_and(|content| !content.trim().is_empty())
        && msg
            .get("toolCalls")
            .and_then(|v| v.as_array())
            .is_none_or(Vec::is_empty)
        && msg
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .is_none_or(Vec::is_empty)
}

/// A message truncation left with no content cannot be delivered.
fn is_emptied_by_truncation(msg: &serde_json::Value) -> bool {
    msg.get("truncated").and_then(|v| v.as_bool()) == Some(true)
        && msg
            .get("content")
            .and_then(|v| v.as_str())
            .is_none_or(str::is_empty)
}

fn report_envelope_size(selected: &[serde_json::Value], next: Option<&serde_json::Value>) -> usize {
    serde_json::to_vec(&serde_json::json!({
        "messages": selected.iter().chain(next).collect::<Vec<_>>(),
        "truncated": false,
        "hasMoreMessages": false,
        "messageContentTruncated": false,
    }))
    .map(|v| v.len())
    .unwrap_or(usize::MAX)
}

fn strip_unbounded_payloads(msg: &mut serde_json::Value) {
    if let Some(obj) = msg.as_object_mut() {
        obj.remove("toolCalls");
        obj.remove("tool_calls");
        obj.remove("imageBlocks");
        obj.remove("image_blocks");
    }
}

/// Cut `msg`'s content to the longest prefix (on a character boundary)
/// that keeps the report envelope within `budget`. The cut is marked
/// `truncated` with the original `contentLength`; no recovery command is
/// offered (#2114).
fn truncate_message_to_fit(
    msg: &mut serde_json::Value,
    selected: &[serde_json::Value],
    budget: usize,
) {
    let Some(original) = msg
        .get("content")
        .and_then(|v| v.as_str())
        .map(str::to_string)
    else {
        msg["truncated"] = serde_json::json!(true);
        return;
    };
    // A length the child already reported (a preview's) is the true one.
    if msg.get("contentLength").is_none() {
        msg["contentLength"] = serde_json::json!(original.len());
    }
    msg["truncated"] = serde_json::json!(true);
    if let Some(obj) = msg.as_object_mut() {
        obj.remove("contentRecovery");
    }
    let mut low = 0usize;
    let mut high = original.len();
    let mut best = 0usize;
    while low <= high {
        let mid = (low + high) / 2;
        let mut end = mid.min(original.len());
        while !original.is_char_boundary(end) {
            end -= 1;
        }
        msg["content"] = serde_json::Value::String(original[..end].to_string());
        if report_envelope_size(selected, Some(msg)) <= budget {
            best = end;
            low = mid.saturating_add(1);
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }
    msg["content"] = serde_json::Value::String(original[..best].to_string());
}
