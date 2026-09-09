//! Result text extraction for tool results (split from `client.rs` for the
//! 750-line cap).

/// Extract the first text content from a tool result JSON value.
///
/// The server sends tool results as:
/// ```json
/// {"content": [{"type": "text", "text": "..."}]}
/// ```
/// This function extracts the `text` field from the first text block.
/// Used by `app.rs` when handling `ToolExecutionEnd` events.
pub fn extract_result_text(result: &serde_json::Value) -> String {
    result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| {
            arr.iter()
                .filter_map(|v| v.get("text").and_then(|t| t.as_str()))
                .next()
        })
        .unwrap_or("")
        .to_string()
}
