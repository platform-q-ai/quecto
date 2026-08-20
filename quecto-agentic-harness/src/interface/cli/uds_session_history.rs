use crate::domain::ids::MessageId;
use crate::domain::message::{Message, ThinkingBlock};

use super::{message_to_json, role_wire_name};

pub(crate) use super::super::protocol::HISTORY_PAGE_SIZE;

pub(crate) const HISTORY_PAGE_JSON_BUDGET: usize =
    crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET / 2;
pub(super) const HISTORY_MESSAGE_SUMMARY_PREVIEW_BYTES: usize = 2048;
pub(super) const HISTORY_THINKING_SUMMARY_PREVIEW_BYTES: usize = 2048;

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
        let mut value = message_to_json_for_history_page(&messages[idx]);
        value["ordinal"] = serde_json::json!(messages[idx].ordinal.unwrap_or((idx + 1) as u64));
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

fn byte_preview(s: &str, max_bytes: usize) -> String {
    s.chars()
        .scan(0usize, |used, ch| {
            let next = *used + ch.len_utf8();
            if next > max_bytes {
                None
            } else {
                *used = next;
                Some(ch)
            }
        })
        .collect()
}

fn thinking_summary_json(msg: &Message, max_encoded_bytes: usize) -> serde_json::Value {
    let mut values = Vec::new();
    let mut used = 2usize;
    let mut omitted = 0usize;
    for block in &msg.thinking_blocks {
        let value = match block {
            ThinkingBlock::Normal { thinking, .. } => {
                let preview = byte_preview(thinking, HISTORY_THINKING_SUMMARY_PREVIEW_BYTES);
                let mut value = serde_json::json!({
                    "kind": "text",
                    "text": preview,
                });
                if value["text"].as_str().map(str::len).unwrap_or(0) < thinking.len() {
                    value["truncated"] = serde_json::json!(true);
                    value["textLength"] = serde_json::json!(thinking.len());
                }
                value
            }
            ThinkingBlock::Redacted { .. } => serde_json::json!({ "kind": "redacted" }),
        };
        let value_len = serde_json::to_vec(&value)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        let next_len = used
            .saturating_add(value_len)
            .saturating_add(!values.is_empty() as usize);
        if next_len > max_encoded_bytes && !values.is_empty() {
            omitted += 1;
            continue;
        }
        used = next_len;
        values.push(value);
    }
    if omitted > 0 {
        values.push(serde_json::json!({
            "kind": "text",
            "text": "",
            "truncated": true,
            "omittedBlocks": omitted,
        }));
    }
    serde_json::Value::Array(values)
}

pub(crate) fn message_to_json_for_history_page(msg: &Message) -> serde_json::Value {
    let full = message_to_json(msg);
    let full_size = serde_json::to_vec(&full)
        .map(|v| v.len())
        .unwrap_or(usize::MAX);
    if full_size <= HISTORY_PAGE_JSON_BUDGET {
        return full;
    }

    let preview = byte_preview(&msg.content, HISTORY_MESSAGE_SUMMARY_PREVIEW_BYTES);

    let mut summary = serde_json::json!({
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
    });
    if !msg.thinking_blocks.is_empty() {
        let base_size = serde_json::to_vec(&summary)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        let thinking_budget = HISTORY_PAGE_JSON_BUDGET.saturating_sub(base_size + 64);
        summary["thinking"] = thinking_summary_json(msg, thinking_budget);
    }
    while serde_json::to_vec(&summary)
        .map(|v| v.len() > HISTORY_PAGE_JSON_BUDGET)
        .unwrap_or(true)
    {
        let Some(thinking) = summary["thinking"].as_array_mut() else {
            break;
        };
        if thinking.len() <= 1 {
            break;
        }
        let removed = thinking.pop().unwrap();
        let omitted = removed
            .get("omittedBlocks")
            .and_then(|value| value.as_u64())
            .unwrap_or(1) as usize;
        if let Some(last) = thinking.last_mut() {
            if last.get("omittedBlocks").is_some() {
                let current = last["omittedBlocks"].as_u64().unwrap_or(0) as usize;
                last["omittedBlocks"] = serde_json::json!(current + omitted);
            } else {
                thinking.push(serde_json::json!({
                    "kind": "text",
                    "text": "",
                    "truncated": true,
                    "omittedBlocks": omitted,
                }));
            }
        }
    }
    summary
}

#[cfg(test)]
#[path = "uds_session_history_budget_tests.rs"]
mod get_messages_history_budget_tests;
