use crate::domain::message::Message;
use crate::interface::cli::protocol::AgentEvent;
use crate::interface::cli::uds_thinking_view::VisibleThinkingBlocksView;

#[cfg(test)]
#[path = "uds_session_message_range/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "uds_session_message_range_cov_tests.rs"]
mod cov_tests;

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

fn message_to_json_with_content_and_thinking(
    msg: &Message,
    content: &str,
    include_thinking: bool,
) -> serde_json::Value {
    let mut value = serde_json::json!({
        "id": msg.id().to_string(),
        "role": super::role_wire_name(&msg.role),
        "content": content,
        "toolCalls": tool_calls_json(msg),
        "toolCallId": msg.tool_call_id,
        "toolName": msg.tool_name,
        "isError": msg.is_error,
        "collapsed": msg.is_collapsed,
    });
    if include_thinking && !msg.thinking_blocks.is_empty() {
        value["thinking"] = serde_json::to_value(VisibleThinkingBlocksView(&msg.thinking_blocks))
            .expect("visible thinking serializes");
    }
    value
}

fn one_char_end(s: &str, start: usize) -> usize {
    s[start..]
        .char_indices()
        .nth(1)
        .map(|(idx, _)| start + idx)
        .unwrap_or(s.len())
}

fn visible_thinking_text(msg: &Message) -> String {
    use crate::domain::message::ThinkingBlock;

    let mut out = String::new();
    for block in &msg.thinking_blocks {
        if let ThinkingBlock::Normal { thinking, .. } = block {
            out.push_str(thinking);
        }
    }
    out
}

fn add_bounded_thinking_page(
    value: &mut serde_json::Value,
    thinking: &str,
    start: usize,
    request_id: Option<&str>,
) {
    if thinking.is_empty() || start >= thinking.len() {
        return;
    }

    let start = nearest_char_boundary_at_or_before(thinking, start);
    let mut end = thinking.len();
    value["thinking"] = serde_json::json!([{ "kind": "text", "text": &thinking[start..end] }]);
    if data_fits_frame(value, request_id) {
        value["thinkingOffset"] = serde_json::json!(start);
        value["nextThinkingOffset"] = serde_json::json!(end);
        value["thinkingLength"] = serde_json::json!(thinking.len());
        value["hasMoreThinking"] = serde_json::json!(end < thinking.len());
        return;
    }

    while end > start {
        let midpoint = start + (end - start) / 2;
        end = nearest_char_boundary_at_or_before(thinking, midpoint);
        if end == start && start < thinking.len() {
            end = one_char_end(thinking, start);
        }
        value["thinking"] = serde_json::json!([{ "kind": "text", "text": &thinking[start..end] }]);
        value["thinkingOffset"] = serde_json::json!(start);
        value["nextThinkingOffset"] = serde_json::json!(end);
        value["thinkingLength"] = serde_json::json!(thinking.len());
        value["hasMoreThinking"] = serde_json::json!(end < thinking.len());
        if data_fits_frame(value, request_id) {
            return;
        }
        if end <= 1 {
            break;
        }
    }

    value.as_object_mut().expect("object").remove("thinking");
    value
        .as_object_mut()
        .expect("object")
        .remove("thinkingOffset");
    value
        .as_object_mut()
        .expect("object")
        .remove("nextThinkingOffset");
    value
        .as_object_mut()
        .expect("object")
        .remove("thinkingLength");
    value
        .as_object_mut()
        .expect("object")
        .remove("hasMoreThinking");
}

fn ranged_value(
    msg: &Message,
    start: usize,
    end: usize,
    content_len: usize,
    thinking_start: usize,
    request_id: Option<&str>,
) -> serde_json::Value {
    let mut value = message_to_json_with_content_and_thinking(msg, &msg.content[start..end], false);
    add_bounded_thinking_page(
        &mut value,
        &visible_thinking_text(msg),
        thinking_start,
        request_id,
    );
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
    while end > start
        && !data_fits_frame(
            &ranged_value(msg, start, end, content_len, 0, request_id),
            request_id,
        )
    {
        let midpoint = start + (end - start) / 2;
        end = nearest_char_boundary_at_or_before(&msg.content, midpoint);
        if end == start {
            end = one_char_end(&msg.content, start);
            break;
        }
    }
    if end > start
        && !data_fits_frame(
            &ranged_value(msg, start, end, content_len, 0, request_id),
            request_id,
        )
    {
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

pub fn tool_call_arguments_to_json_range_for_response(
    msg: &Message,
    tool_call_id: &str,
    offset: Option<usize>,
    limit: Option<usize>,
    request_id: Option<&str>,
) -> Option<serde_json::Value> {
    let tool_call = msg.tool_calls.iter().find(|call| call.id == tool_call_id)?;
    let arguments = &tool_call.arguments;
    let arguments_len = arguments.len();
    let start = nearest_char_boundary_at_or_before(arguments, offset.unwrap_or(0));
    let remaining = arguments_len.saturating_sub(start);
    let requested = limit.unwrap_or(remaining).min(remaining);
    let requested_end = nearest_char_boundary_at_or_before(
        arguments,
        start.saturating_add(requested).min(arguments_len),
    );
    let mut end = requested_end;
    if end == start && start < arguments_len {
        end = one_char_end(arguments, start);
    }
    let build = |end| {
        serde_json::json!({
            "id": msg.id().to_string(),
            "toolCallId": tool_call.id,
            "toolName": tool_call.name,
            "arguments": &arguments[start..end],
            "offset": start,
            "nextOffset": end,
            "argumentsLength": arguments_len,
            "hasMoreArguments": end < arguments_len,
        })
    };
    while end > start && !data_fits_frame(&build(end), request_id) {
        let midpoint = start + (end - start) / 2;
        end = nearest_char_boundary_at_or_before(arguments, midpoint);
        if end == start {
            end = one_char_end(arguments, start);
            break;
        }
    }
    if !data_fits_frame(&build(end), request_id) {
        return None;
    }
    Some(build(end))
}

pub fn message_to_json_range_for_response(
    msg: &Message,
    offset: Option<usize>,
    limit: Option<usize>,
    request_id: Option<&str>,
) -> serde_json::Value {
    if offset.is_none() && limit.is_none() {
        let value = super::message_to_json(msg);
        if data_fits_frame(&value, request_id) {
            return value;
        }
    }

    let content_len = msg.content.len();
    let start = nearest_char_boundary_at_or_before(&msg.content, offset.unwrap_or(0));
    let remaining = content_len.saturating_sub(start);
    let requested = limit.unwrap_or(remaining).min(remaining);
    let requested_end = start.saturating_add(requested).min(content_len);
    let end = bounded_range_end(msg, start, requested_end, content_len, request_id);
    ranged_value(
        msg,
        start,
        end,
        content_len,
        offset.unwrap_or(0),
        request_id,
    )
}
