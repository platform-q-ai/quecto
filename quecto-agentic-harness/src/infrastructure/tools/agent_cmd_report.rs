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
        if report_envelope_size(&selected, Some(&msg)) <= REPORT_BUDGET_BYTES || selected.is_empty()
        {
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

fn is_substantive_assistant(msg: &serde_json::Value) -> bool {
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
        if let Some(id) = msg.get("id").and_then(|v| v.as_str()).map(str::to_owned) {
            msg["contentRecovery"] = serde_json::json!({
                "command": "get_message",
                "messageId": id,
                "offset": end,
            });
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
    if let Some(id) = msg.get("id").and_then(|v| v.as_str()).map(str::to_owned) {
        msg["contentRecovery"] = serde_json::json!({
            "command": "get_message",
            "messageId": id,
            "offset": best,
        });
    }
}
