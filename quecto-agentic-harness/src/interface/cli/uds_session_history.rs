//! Presenter of a history page (#1856, #1971): the `get_messages` wire
//! shape over the application's [`HistoryPage`], and the transport byte
//! budget that decides how many of the page's messages one frame carries.
//!
//! Which messages a page holds, its cursor and `hasMoreBefore` are the
//! application's (`ReadHistory`); this module only encodes them, trims the
//! oldest of them when the frame budget would overflow (through
//! [`HistoryPage::keeping_newest`], so the cursor rule stays one), and
//! summarises a single message too large to carry whole.
use crate::application::sessions::dto::HistoryPage;
use crate::domain::message::{Message, ThinkingBlock};

use super::{message_to_json, role_wire_name};

pub(crate) use super::super::protocol::HISTORY_PAGE_SIZE;

pub(crate) const HISTORY_PAGE_JSON_BUDGET: usize =
    crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET / 2;
pub(super) const HISTORY_MESSAGE_SUMMARY_PREVIEW_BYTES: usize = 2048;
pub(super) const HISTORY_THINKING_SUMMARY_PREVIEW_BYTES: usize = 2048;

/// The `get_messages` response data for `page`: its messages in
/// chronological order under the frame budget, the `before` cursor and
/// `hasMoreBefore`.
///
/// The budget is applied newest-first so the newest messages always
/// arrive; a single message that alone exceeds the budget is carried as a
/// recoverable summary (#1107). When older messages of the window are
/// dropped, the page's cursor moves to the oldest kept one.
pub(crate) fn history_page_json(page: HistoryPage) -> serde_json::Value {
    let mut encoded: Vec<serde_json::Value> = Vec::new();
    let mut used = 0usize;
    for message in page.messages.iter().rev() {
        let mut value = message_to_json_for_history_page(message);
        value["ordinal"] = serde_json::json!(message.ordinal);
        let sz = serde_json::to_vec(&value)
            .map(|v| v.len())
            .unwrap_or(usize::MAX)
            + 1;
        if !encoded.is_empty() && used.saturating_add(sz) > HISTORY_PAGE_JSON_BUDGET {
            break;
        }
        used = used.saturating_add(sz);
        encoded.push(value);
    }
    encoded.reverse();
    let page = page.keeping_newest(encoded.len());
    serde_json::json!({
        "messages": encoded,
        "before": page.before.as_ref().map(|id| id.as_str().to_string()),
        "hasMoreBefore": page.has_more_before,
    })
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
    // JSON never shrinks a string: a body or argument payload already past
    // the budget is over it without serialising the whole message first.
    let payloads = std::iter::once(msg.content.len())
        .chain(msg.tool_calls.iter().map(|call| call.arguments.len()));
    if payloads.max().unwrap_or(0) <= HISTORY_PAGE_JSON_BUDGET {
        let full = message_to_json(msg);
        let encoded = serde_json::to_vec(&full).map(|v| v.len());
        if encoded.unwrap_or(usize::MAX) <= HISTORY_PAGE_JSON_BUDGET {
            return full;
        }
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

/// Test rig: the `get_messages` data for `messages` paged through the
/// composed history use case over an ephemeral read model.
#[cfg(test)]
pub(crate) fn messages_page_json(
    messages: &[Message],
    count: usize,
    before: Option<&str>,
) -> serde_json::Value {
    let handles =
        crate::interface::cli::uds::dispatch_session_roster_tests::ephemeral_read_handles(&[]);
    history_page_json(
        handles
            .read_history
            .page(messages, "", count, before)
            .expect("test pages carry known cursors"),
    )
}

#[cfg(test)]
#[path = "uds_session_history_budget_tests.rs"]
mod get_messages_history_budget_tests;
