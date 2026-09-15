//! Presenter of recovered content (#1858, #1971): the `get_message` wire
//! shape over the application's [`RecoveredContent`], and the protocol
//! frame cap that decides how much of the selected range one response
//! carries.
//!
//! Which message and which range are the application's
//! (`RecoverMessage`, [`Utf8Range`]); this module only encodes them and
//! narrows a range — through [`Utf8Range::halve`], so character boundaries
//! stay one rule — until the framed response fits (#1094), with range
//! metadata so clients walk and reassemble content that exceeds the cap.
use crate::application::sessions::dto::{RecoveredContent, Utf8Range};
use crate::domain::message::{Message, ToolCall};
use crate::domain::visible_thinking::{visible_thinking_len, visible_thinking_page};
use crate::interface::cli::protocol::AgentEvent;

#[cfg(test)]
#[path = "uds_session_message_range/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "uds_session_message_range_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
const LONG_REQUEST_ID_REGRESSION_LEN: usize = 4096;

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
        value["thinking"] =
            super::uds_visible_thinking_wire::visible_thinking_blocks_json(&msg.thinking_blocks);
    }
    value
}

fn clear_thinking_page(value: &mut serde_json::Value) {
    let object = value.as_object_mut().expect("object");
    object.remove("thinking");
    object.remove("thinkingOffset");
    object.remove("nextThinkingOffset");
    object.remove("thinkingLength");
    object.remove("hasMoreThinking");
}

fn add_bounded_thinking_page(
    value: &mut serde_json::Value,
    msg: &Message,
    start: usize,
    request_id: Option<&str>,
) {
    let thinking_len = visible_thinking_len(&msg.thinking_blocks);
    if thinking_len == 0 || start >= thinking_len {
        return;
    }

    let start = start.min(thinking_len);
    let mut end = thinking_len;
    value["thinking"] = super::uds_visible_thinking_wire::visible_thinking_page_json(
        visible_thinking_page(&msg.thinking_blocks, start, end),
    );
    if data_fits_frame(value, request_id) {
        value["thinkingOffset"] = serde_json::json!(start);
        value["nextThinkingOffset"] = serde_json::json!(end);
        value["thinkingLength"] = serde_json::json!(thinking_len);
        value["hasMoreThinking"] = serde_json::json!(end < thinking_len);
        return;
    }

    while end > start {
        let midpoint = start + (end - start) / 2;
        end = midpoint;
        if end == start && start < thinking_len {
            end = (start + 1).min(thinking_len);
        }
        value["thinking"] = super::uds_visible_thinking_wire::visible_thinking_page_json(
            visible_thinking_page(&msg.thinking_blocks, start, end),
        );
        value["thinkingOffset"] = serde_json::json!(start);
        value["nextThinkingOffset"] = serde_json::json!(end);
        value["thinkingLength"] = serde_json::json!(thinking_len);
        value["hasMoreThinking"] = serde_json::json!(end < thinking_len);
        if data_fits_frame(value, request_id) {
            return;
        }
        if end <= start + 1 {
            break;
        }
    }

    clear_thinking_page(value);
}

fn ranged_value(
    msg: &Message,
    range: Utf8Range,
    thinking_start: usize,
    request_id: Option<&str>,
) -> serde_json::Value {
    let mut value =
        message_to_json_with_content_and_thinking(msg, range.slice(&msg.content), false);
    add_bounded_thinking_page(&mut value, msg, thinking_start, request_id);
    value["offset"] = serde_json::json!(range.start);
    value["nextOffset"] = serde_json::json!(range.end);
    value["contentLength"] = serde_json::json!(msg.content.len());
    value["hasMoreContent"] = serde_json::json!(range.end < msg.content.len());
    value
}

fn data_fits_frame(value: &serde_json::Value, request_id: Option<&str>) -> bool {
    let response = AgentEvent::ok(request_id, "get_message", Some(value.clone()));
    response.to_json_line().len() <= crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET
}

/// Narrow the requested content range until the framed response fits.
/// The fit is probed without a thinking page; an unframeable single
/// character yields the empty range at `start`.
fn framed_content_range(
    msg: &Message,
    requested: Utf8Range,
    request_id: Option<&str>,
) -> Utf8Range {
    let mut range = requested;
    while !range.is_empty()
        && !data_fits_frame(&ranged_value(msg, range, 0, request_id), request_id)
    {
        if !range.halve(&msg.content) {
            break;
        }
    }
    if !range.is_empty() && !data_fits_frame(&ranged_value(msg, range, 0, request_id), request_id) {
        range.end = range.start;
    }
    range
}

fn message_range_json(
    msg: &Message,
    requested: Utf8Range,
    thinking_offset: usize,
    ranged: bool,
    request_id: Option<&str>,
) -> serde_json::Value {
    if !ranged {
        let value = super::message_to_json(msg);
        if data_fits_frame(&value, request_id) {
            return value;
        }
    }
    let range = framed_content_range(msg, requested, request_id);
    ranged_value(msg, range, thinking_offset, request_id)
}

fn tool_call_arguments_json(
    message_id: &str,
    tool_call: &ToolCall,
    requested: Utf8Range,
    request_id: Option<&str>,
) -> Option<serde_json::Value> {
    let arguments = &tool_call.arguments;
    let build = |range: Utf8Range| {
        serde_json::json!({
            "id": message_id,
            "toolCallId": tool_call.id,
            "toolName": tool_call.name,
            "arguments": range.slice(arguments),
            "offset": range.start,
            "nextOffset": range.end,
            "argumentsLength": arguments.len(),
            "hasMoreArguments": range.end < arguments.len(),
        })
    };
    let mut range = requested;
    while !range.is_empty() && !data_fits_frame(&build(range), request_id) {
        if !range.halve(arguments) {
            break;
        }
    }
    if !data_fits_frame(&build(range), request_id) {
        return None;
    }
    Some(build(range))
}

/// The `get_message` response data for `content`, framed under the
/// protocol cap: a whole message when unranged and it fits, else the
/// requested content (or tool-call argument) range narrowed to fit, with
/// `offset`/`nextOffset`/length/`hasMore…` metadata. `None` when even one
/// character of tool-call arguments cannot be framed.
pub(crate) fn recovered_content_json(
    content: &RecoveredContent,
    request_id: Option<&str>,
) -> Option<serde_json::Value> {
    match content {
        RecoveredContent::Message {
            message,
            range,
            thinking_offset,
            ranged,
        } => Some(message_range_json(
            message,
            *range,
            *thinking_offset,
            *ranged,
            request_id,
        )),
        RecoveredContent::ToolCallArguments {
            message_id,
            tool_call,
            range,
        } => tool_call_arguments_json(message_id.as_str(), tool_call, *range, request_id),
    }
}

/// Test rig: `get_message` data for `msg` with the given range fields.
#[cfg(test)]
pub(crate) fn message_to_json_range_for_response(
    msg: &Message,
    offset: Option<usize>,
    thinking_offset: Option<usize>,
    limit: Option<usize>,
    request_id: Option<&str>,
) -> serde_json::Value {
    use crate::application::sessions::dto::ContentSelector;
    use crate::application::sessions::use_cases::RecoverMessage;
    let content = RecoverMessage::select(
        msg.clone(),
        &ContentSelector::Message {
            offset,
            thinking_offset,
            limit,
        },
    )
    .expect("a message selector never fails");
    recovered_content_json(&content, request_id).expect("message content always frames")
}

/// Test rig: `get_message` data for one tool call's arguments.
#[cfg(test)]
pub(crate) fn tool_call_arguments_to_json_range_for_response(
    msg: &Message,
    tool_call_id: &str,
    offset: Option<usize>,
    limit: Option<usize>,
    request_id: Option<&str>,
) -> Option<serde_json::Value> {
    use crate::application::sessions::dto::ContentSelector;
    use crate::application::sessions::use_cases::RecoverMessage;
    let content = RecoverMessage::select(
        msg.clone(),
        &ContentSelector::ToolCallArguments {
            tool_call_id: tool_call_id.into(),
            offset,
            limit,
        },
    )
    .ok()?;
    recovered_content_json(&content, request_id)
}
