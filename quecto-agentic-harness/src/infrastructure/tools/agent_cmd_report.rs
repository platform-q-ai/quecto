use crate::domain::session::PendingMessageReport;

pub(crate) fn mint_default_report_receipt() -> String {
    format!("agent-cmd-report-{}", uuid::Uuid::new_v4())
}

pub(crate) fn delivery_receipt(response: &serde_json::Value) -> Option<&str> {
    response
        .pointer("/data/deliveryReceipt")
        .and_then(|v| v.as_str())
        .filter(|receipt| !receipt.is_empty())
}

pub(crate) fn pending_delivery_match_index(
    pending: &std::collections::VecDeque<PendingMessageReport>,
    receipt: Option<&str>,
    delivered_content: &str,
) -> Option<usize> {
    if let Some(receipt) = receipt {
        return pending
            .iter()
            .position(|pending| !pending.receipt.is_empty() && pending.receipt == receipt);
    }
    let mut matches = pending
        .iter()
        .enumerate()
        .filter(|(_, pending)| pending.receipt.is_empty() && pending.response == delivered_content);
    let (index, _) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(index)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DefaultReportPlan {
    pub content: String,
    pub pending: Option<PendingMessageReport>,
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
    let observed_max = messages
        .iter()
        .filter_map(|m| m.get("ordinal").and_then(|v| v.as_u64()))
        .max()
        .unwrap_or(0);
    if report_incomplete {
        if observed_max > delivered {
            let report = bounded_report_messages(
                messages
                    .iter()
                    .filter(|m| {
                        m.get("ordinal")
                            .and_then(|v| v.as_u64())
                            .is_some_and(|ord| ord > delivered)
                    })
                    .cloned()
                    .collect(),
                observed_max,
            );
            if !report.messages.is_empty() {
                *data = serde_json::json!({"messages": report.messages, "truncated": true, "hasMoreMessages": report.has_more_messages, "messageContentTruncated": report.message_content_truncated, "reportIncomplete": true});
                return unchanged(envelope.to_string());
            }
        }
        *data = serde_json::json!({"unchanged": true, "reportIncomplete": true});
        return unchanged(envelope.to_string());
    }
    if observed_max < delivered {
        *data = serde_json::json!({"unchanged": true});
        return unchanged(envelope.to_string());
    }
    let mut max_ord = delivered;
    let mut latest_assistant = None;
    let mut unread = Vec::new();
    for (idx, msg) in messages.iter_mut().enumerate() {
        let ord = msg
            .get("ordinal")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| {
                let next = max_ord.saturating_add(1);
                msg["ordinal"] = serde_json::json!(next);
                next
            });
        max_ord = max_ord.max(ord);
        if ord > delivered {
            unread.push(idx);
            if is_substantive_assistant(msg) {
                latest_assistant = Some(idx);
            }
        }
    }
    if unread.is_empty() {
        *data = serde_json::json!({"unchanged": true});
        return unchanged(envelope.to_string());
    }
    let candidates: Vec<_> = if delivered == 0 {
        latest_assistant
            .into_iter()
            .map(|i| messages[i].clone())
            .collect()
    } else {
        unread.into_iter().map(|i| messages[i].clone()).collect()
    };
    if candidates.is_empty() {
        *data = serde_json::json!({"unchanged": true});
        return unchanged(envelope.to_string());
    }
    let report = bounded_report_messages(candidates, max_ord);
    if report.messages.is_empty() {
        *data = if max_ord > delivered {
            serde_json::json!({"messages": [], "truncated": true, "hasMoreMessages": true, "messageContentTruncated": false, "reportIncomplete": true})
        } else {
            serde_json::json!({"unchanged": true})
        };
        return unchanged(envelope.to_string());
    }
    let ordinal = report
        .messages
        .iter()
        .filter_map(|m| m.get("ordinal").and_then(|v| v.as_u64()))
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
    pending_delivery_match_index(pending, receipt, content)
        .map(DeliveryDecision::Acknowledge)
        .unwrap_or(DeliveryDecision::Ignore)
}

pub(crate) fn needs_default_report_backfill(
    messages: &[serde_json::Value],
    delivered: u64,
) -> bool {
    if delivered == 0
        && messages
            .iter()
            .any(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"))
    {
        return false;
    }
    messages
        .first()
        .and_then(|m| m.get("ordinal").and_then(|v| v.as_u64()))
        .is_some_and(|ord| ord > delivered.saturating_add(1))
}

pub(crate) const REPORT_BUDGET_BYTES: usize = 800 * 4;

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

    // Preserve transcript order when it already fits. If it does not, reserve the
    // envelope for the latest substantive assistant handoff before optional context.
    let all_fit = report_envelope_size(&candidates, None) <= REPORT_BUDGET_BYTES;
    let mut ordered = if all_fit {
        candidates
    } else if let Some(final_idx) = candidates.iter().rposition(is_substantive_assistant) {
        let final_message = candidates.remove(final_idx);
        let mut prioritized = vec![final_message];
        prioritized.extend(candidates.into_iter().rev());
        prioritized
    } else {
        candidates
    };

    let candidate_count = ordered.len();
    let mut selected = Vec::new();
    for mut msg in ordered.drain(..) {
        if report_envelope_size(&selected, Some(&msg)) > REPORT_BUDGET_BYTES {
            truncate_message_to_fit(&mut msg, &selected);
        }
        if is_unrecoverably_truncated(&msg) {
            break;
        }
        if report_envelope_size(&selected, Some(&msg)) <= REPORT_BUDGET_BYTES {
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

fn is_unrecoverably_truncated(msg: &serde_json::Value) -> bool {
    msg.get("truncated").and_then(|v| v.as_bool()) == Some(true)
        && !has_usable_content_recovery(msg)
}

fn has_usable_content_recovery(msg: &serde_json::Value) -> bool {
    let Some(recovery) = msg.get("contentRecovery") else {
        return false;
    };
    recovery.get("command").and_then(|v| v.as_str()) == Some("get_message")
        && recovery
            .get("messageId")
            .and_then(|v| v.as_str())
            .is_some_and(|id| !id.is_empty())
        && recovery
            .get("offset")
            .and_then(|v| v.as_u64())
            .is_some_and(|offset| offset > 0)
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

fn truncate_message_to_fit(msg: &mut serde_json::Value, selected: &[serde_json::Value]) {
    let original_len = msg.get("content").and_then(|v| v.as_str()).map(str::len);
    let Some(original) = msg
        .get("content")
        .and_then(|v| v.as_str())
        .map(str::to_string)
    else {
        msg["truncated"] = serde_json::json!(true);
        return;
    };
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
        if let Some(len) = original_len {
            msg["contentLength"] = serde_json::json!(len);
        }
        msg["truncated"] = serde_json::json!(true);
        if end > 0 {
            if let Some(id) = msg.get("id").and_then(|v| v.as_str()).map(str::to_owned) {
                msg["contentRecovery"] = serde_json::json!({
                    "command": "get_message",
                    "messageId": id,
                    "offset": end,
                });
            }
        } else if let Some(obj) = msg.as_object_mut() {
            obj.remove("contentRecovery");
        }
        if report_envelope_size(selected, Some(msg)) <= REPORT_BUDGET_BYTES {
            best = end;
            low = mid.saturating_add(1);
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }
    msg["content"] = serde_json::Value::String(original[..best].to_string());
    if best > 0 {
        if let Some(id) = msg.get("id").and_then(|v| v.as_str()).map(str::to_owned) {
            msg["contentRecovery"] = serde_json::json!({
                "command": "get_message",
                "messageId": id,
                "offset": best,
            });
        }
    } else if let Some(obj) = msg.as_object_mut() {
        obj.remove("contentRecovery");
    }
}
