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

pub(crate) fn bounded_report_messages(
    mut candidates: Vec<serde_json::Value>,
    max_available_ordinal: u64,
) -> (Vec<serde_json::Value>, bool) {
    let mut selected = Vec::new();
    for mut msg in candidates.drain(..) {
        strip_unbounded_payloads(&mut msg);
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
    let committed = selected
        .iter()
        .filter_map(|m| m.get("ordinal").and_then(|v| v.as_u64()))
        .max()
        .unwrap_or(0);
    let truncated = selected
        .iter()
        .any(|m| m.get("truncated").and_then(|v| v.as_bool()) == Some(true))
        || committed < max_available_ordinal;
    (selected, truncated)
}

fn report_envelope_size(selected: &[serde_json::Value], next: Option<&serde_json::Value>) -> usize {
    serde_json::to_vec(&serde_json::json!({
        "messages": selected.iter().chain(next).collect::<Vec<_>>(),
        "truncated": false
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
        if report_envelope_size(selected, Some(msg)) <= REPORT_BUDGET_BYTES {
            best = end;
            low = mid.saturating_add(1);
        } else {
            if mid == 0 {
                break;
            }
            high = mid - 1;
        }
    }
    msg["content"] = serde_json::Value::String(original[..best].to_string());
}
