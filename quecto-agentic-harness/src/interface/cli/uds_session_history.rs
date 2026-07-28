use crate::domain::ids::MessageId;
use crate::domain::message::Message;

use super::{message_to_json, role_wire_name};

pub(crate) use super::super::protocol::HISTORY_PAGE_SIZE;

pub(crate) const HISTORY_PAGE_JSON_BUDGET: usize =
    crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET / 2;
pub(super) const HISTORY_MESSAGE_SUMMARY_PREVIEW_BYTES: usize = 2048;

/// Locate a message by its stable wire id (a stringified UUID). Parses the id
/// ONCE and compares typed UUIDs instead of allocating a `to_string` per
/// candidate (#1061 review). A non-UUID id matches nothing, so `None`.
pub(crate) fn position_by_message_id(
    messages: &[Message],
    message_id: &MessageId,
) -> Option<usize> {
    let target = uuid::Uuid::parse_str(message_id.as_str()).ok()?;
    messages.iter().position(|m| m.id() == target)
}

pub(crate) fn position_by_wire_id(messages: &[Message], wire_id: &str) -> Option<usize> {
    position_by_message_id(messages, &MessageId::from(wire_id))
}

/// Return a JSON value containing the selected history window in chronological order.
///
/// `count: 0` keeps the legacy empty-page contract and reports no cursor (the
/// cursor names the oldest INCLUDED message, which an empty window lacks).
pub(crate) fn messages_page_json_for_id(
    messages: &[Message],
    count: usize,
    before: Option<&MessageId>,
) -> serde_json::Value {
    let end = before
        .and_then(|cursor| position_by_message_id(messages, cursor))
        .unwrap_or(messages.len());
    // Default callers supply HISTORY_PAGE_SIZE; an explicit `count` retains the
    // legacy "last N" contract (including counts above one page), subject to the
    // byte budget below so a single huge tool result cannot flood callers (#1107).
    let nominal_start = end.saturating_sub(count);
    let mut selected: Vec<serde_json::Value> = Vec::new();
    let mut selected_start = end;
    let mut used = 0usize;
    for idx in (nominal_start..end).rev() {
        let value = message_to_json_for_history_page(&messages[idx]);
        let sz = serde_json::to_vec(&value)
            .map(|v| v.len())
            .unwrap_or(usize::MAX)
            + 1;
        if !selected.is_empty() && used.saturating_add(sz) > HISTORY_PAGE_JSON_BUDGET {
            break;
        }
        used = used.saturating_add(sz);
        selected_start = idx;
        selected.push(value);
        if selected.len() >= count {
            break;
        }
    }
    selected.reverse();
    let has_more_before = count > 0 && selected_start > 0;
    let before_cursor = has_more_before.then(|| messages[selected_start].id().to_string());
    serde_json::json!({
        "messages": selected,
        "before": before_cursor,
        "hasMoreBefore": has_more_before,
    })
}

/// Return a JSON value containing the last `count` messages in chronological order.
pub fn messages_tail_json(messages: &[Message], count: usize) -> serde_json::Value {
    messages_page_json_for_id(messages, count, None)
}

pub fn messages_page_json(
    messages: &[Message],
    count: usize,
    before: Option<&str>,
) -> serde_json::Value {
    let before = before.map(MessageId::from);
    messages_page_json_for_id(messages, count, before.as_ref())
}

pub(crate) fn message_to_json_for_history_page(msg: &Message) -> serde_json::Value {
    let full = message_to_json(msg);
    let full_size = serde_json::to_vec(&full)
        .map(|v| v.len())
        .unwrap_or(usize::MAX);
    if full_size <= HISTORY_PAGE_JSON_BUDGET {
        return full;
    }

    let preview: String = msg
        .content
        .chars()
        .scan(0usize, |used, ch| {
            let next = *used + ch.len_utf8();
            if next > HISTORY_MESSAGE_SUMMARY_PREVIEW_BYTES {
                None
            } else {
                *used = next;
                Some(ch)
            }
        })
        .collect();

    serde_json::json!({
        "id": msg.id().to_string(),
        "role": role_wire_name(&msg.role),
        "content": preview,
        "toolCalls": msg.tool_calls.iter().map(|call| serde_json::json!({
            "id": call.id,
            "name": call.name,
            "arguments": "",
            "argumentsLength": call.arguments.len(),
            "truncated": true,
        })).collect::<Vec<_>>(),
        "toolCallId": msg.tool_call_id,
        "toolName": msg.tool_name,
        "isError": msg.is_error,
        "collapsed": true,
        "truncated": true,
        "contentLength": msg.content.len(),
    })
}

#[cfg(test)]
#[path = "uds_session_history_budget_tests.rs"]
mod get_messages_history_budget_tests;
