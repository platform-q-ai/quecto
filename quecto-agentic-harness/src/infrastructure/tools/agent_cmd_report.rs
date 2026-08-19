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

pub(crate) fn bounded_report_messages(
    mut candidates: Vec<serde_json::Value>,
    max_available_ordinal: u64,
) -> (Vec<serde_json::Value>, bool) {
    const REPORT_BUDGET_BYTES: usize = 800 * 4;
    let mut selected = Vec::new();
    for mut msg in candidates.drain(..) {
        let projected_size = serde_json::to_vec(&serde_json::json!({
            "messages": selected.iter().chain(std::iter::once(&msg)).collect::<Vec<_>>(),
            "truncated": false
        }))
        .map(|v| v.len())
        .unwrap_or(usize::MAX);
        if projected_size <= REPORT_BUDGET_BYTES || selected.is_empty() {
            if projected_size > REPORT_BUDGET_BYTES {
                let original_len = msg.get("content").and_then(|v| v.as_str()).map(str::len);
                if let Some(content) = msg.get_mut("content").and_then(|v| v.as_str()) {
                    let mut end = REPORT_BUDGET_BYTES.min(content.len());
                    while !content.is_char_boundary(end) {
                        end -= 1;
                    }
                    let preview = content[..end].to_string();
                    msg["content"] = serde_json::Value::String(preview);
                    if let Some(len) = original_len {
                        msg["contentLength"] = serde_json::json!(len);
                    }
                    msg["truncated"] = serde_json::json!(true);
                }
            }
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
