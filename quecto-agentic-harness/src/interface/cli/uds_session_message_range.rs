use crate::domain::message::Message;
use crate::interface::cli::protocol::AgentEvent;

#[cfg(test)]
#[path = "uds_session_message_range/tests.rs"]
mod tests;

#[cfg(test)]
const LONG_REQUEST_ID_REGRESSION_LEN: usize = 4096;

fn nearest_char_boundary_at_or_before(s: &str, mut idx: usize) -> usize {
    idx = idx.min(s.len());
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn tool_calls_json(msg: &Message) -> serde_json::Value {
    serde_json::Value::Array(
        msg.tool_calls
            .iter()
            .map(|tc| {
                serde_json::json!({
                    "id": tc.id,
                    "name": tc.name,
                    "arguments": tc.arguments,
                })
            })
            .collect(),
    )
}

fn message_to_json_with_content(msg: &Message, content: &str) -> serde_json::Value {
    serde_json::json!({
        "id": msg.id().to_string(),
        "role": super::role_wire_name(&msg.role),
        "content": content,
        "toolCalls": tool_calls_json(msg),
        "toolCallId": msg.tool_call_id,
        "toolName": msg.tool_name,
        "isError": msg.is_error,
        "collapsed": msg.is_collapsed,
    })
}

fn one_char_end(s: &str, start: usize) -> usize {
    s[start..]
        .char_indices()
        .nth(1)
        .map(|(idx, _)| start + idx)
        .unwrap_or(s.len())
}

fn ranged_value(msg: &Message, start: usize, end: usize, content_len: usize) -> serde_json::Value {
    let mut value = message_to_json_with_content(msg, &msg.content[start..end]);
    value["offset"] = serde_json::json!(start);
    value["nextOffset"] = serde_json::json!(end);
    value["contentLength"] = serde_json::json!(content_len);
    value["hasMoreContent"] = serde_json::json!(end < content_len);
    value
}

fn data_fits_frame(value: &serde_json::Value, request_id: Option<&str>) -> bool {
    let response = AgentEvent::ok(request_id, "get_message", Some(value.clone()));
    response.to_json_line().len() <= crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET
}

fn bounded_range_end(
    msg: &Message,
    start: usize,
    requested_end: usize,
    content_len: usize,
    request_id: Option<&str>,
) -> usize {
    let mut end = nearest_char_boundary_at_or_before(&msg.content, requested_end);
    if end == start && start < content_len {
        end = one_char_end(&msg.content, start);
    }
    while end > start && !data_fits_frame(&ranged_value(msg, start, end, content_len), request_id) {
        let midpoint = start + (end - start) / 2;
        end = nearest_char_boundary_at_or_before(&msg.content, midpoint);
        if end == start {
            end = one_char_end(&msg.content, start);
            break;
        }
    }
    if end > start && !data_fits_frame(&ranged_value(msg, start, end, content_len), request_id) {
        // Metadata/tool-call overhead alone is too large for a success response.
        // Return an empty page rather than pretending progress is frame-safe; the
        // outer response guard will emit the explicit frame-limit error.
        start
    } else {
        end
    }
}

/// Serialize a message for `get_message`, optionally returning only a bounded
/// content byte range. Range metadata lets clients walk and reassemble content
/// that would exceed the protocol frame cap (#1094).
pub fn message_to_json_range(
    msg: &Message,
    offset: Option<usize>,
    limit: Option<usize>,
) -> serde_json::Value {
    message_to_json_range_for_response(msg, offset, limit, None)
}

pub fn message_to_json_range_for_response(
    msg: &Message,
    offset: Option<usize>,
    limit: Option<usize>,
    request_id: Option<&str>,
) -> serde_json::Value {
    if offset.is_none() && limit.is_none() {
        return super::message_to_json(msg);
    }

    let content_len = msg.content.len();
    let start = nearest_char_boundary_at_or_before(&msg.content, offset.unwrap_or(0));
    let remaining = content_len.saturating_sub(start);
    let requested = limit.unwrap_or(remaining).min(remaining);
    let requested_end = start.saturating_add(requested).min(content_len);
    let end = bounded_range_end(msg, start, requested_end, content_len, request_id);
    ranged_value(msg, start, end, content_len)
}
