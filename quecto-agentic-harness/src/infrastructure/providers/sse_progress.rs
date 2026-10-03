//! Which SSE events carry output (#2433).
//!
//! A Codex reply once streamed ~11 recognised events a second for 15
//! minutes with no output at all: every event restarted the stream idle
//! bound, so nothing ended it short of an outer deadline. The stream
//! progress bound ([`super::stream_idle::EventIdle`]) is restarted only by
//! an event this module says carries output — on any of the three wires,
//! read by shape, so the bound needs no vendor:
//!
//! - a generated delta: a Responses `delta` string (text, tool-call
//!   arguments, refusal, reasoning or reasoning summary); a Messages
//!   `delta` with `text`, `thinking` or `partial_json`; an OpenAI chat
//!   choice's `content`, `refusal`, `reasoning`, `reasoning_content` or
//!   `tool_calls`;
//! - a finished part that carries content: a Responses `text`,
//!   `arguments` or `refusal`, an output `item` holding arguments or a
//!   content or summary part with text, a content `part` with text; a
//!   Messages `content_block` holding text — a tool call just opened, or an
//!   OpenAI chat call naming no function and no arguments, is none yet;
//! - completion: a terminal event, an OpenAI chat `finish_reason`, a
//!   Messages `stop_reason`, or `[DONE]`.
//!
//! An empty string or list is no output, so a stream of empty deltas makes
//! no progress. Reasoning a model streams is output, so a model that keeps
//! reasoning is never cut short by this bound; one that thinks silently, or
//! sends only the odd `ping` while it does, is bounded by the idle bound
//! instead ([`super::stream_idle::PROGRESS_EVENTS`]).
use serde_json::Value;

/// The events that end a reply: their arrival is completion.
const TERMINAL_TYPES: &[&str] = &[
    "response.completed",
    "response.failed",
    "response.incomplete",
    "error",
    "message_stop",
];

/// Whether the SSE event whose data is `data` carries output.
pub(crate) fn carries_output(data: &str) -> bool {
    if super::sse_end::is_done_marker(data.trim()) {
        return true;
    }
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return false;
    };
    value["type"]
        .as_str()
        .is_some_and(|kind| TERMINAL_TYPES.contains(&kind))
        || responses(&value)
        || messages(&value)
        || value["choices"]
            .as_array()
            .is_some_and(|choices| choices.iter().any(chat_choice))
}

/// A string or list with something in it.
fn filled(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        _ => false,
    }
}

/// A Responses event that carries output.
fn responses(value: &Value) -> bool {
    let item = &value["item"];
    ["delta", "text", "arguments", "refusal"]
        .iter()
        .any(|field| value[*field].as_str().is_some_and(|text| !text.is_empty()))
        || filled(&item["arguments"])
        || ["content", "summary"].iter().any(|field| {
            item[*field]
                .as_array()
                .is_some_and(|parts| parts.iter().any(part))
        })
        || part(&value["part"])
}

/// A content or summary part holding text or a refusal.
fn part(part: &Value) -> bool {
    filled(&part["text"]) || filled(&part["refusal"])
}

/// A Messages event that carries output.
fn messages(value: &Value) -> bool {
    let delta = &value["delta"];
    let block = &value["content_block"];
    ["text", "thinking", "partial_json"]
        .iter()
        .any(|field| filled(&delta[*field]))
        || delta["stop_reason"].is_string()
        || ["text", "thinking"]
            .iter()
            .any(|field| filled(&block[*field]))
}

/// An OpenAI chat choice that carries output.
fn chat_choice(choice: &Value) -> bool {
    let delta = &choice["delta"];
    ["content", "refusal", "reasoning", "reasoning_content"]
        .iter()
        .any(|field| filled(&delta[*field]))
        || delta["tool_calls"].as_array().is_some_and(|calls| {
            calls.iter().any(|call| {
                ["name", "arguments"]
                    .iter()
                    .any(|field| filled(&call["function"][*field]))
            })
        })
        || choice["finish_reason"].is_string()
}

/// The type suffixes of events that carry output when long: deltas and
/// finished parts (#2433 review).
const LONG_OUTPUT_SUFFIXES: &[&str] = &[".delta", "_delta", ".done"];

/// Whether an event too long to read whole carries output, judged by the
/// type the kept `start` of its data names (#2433 review): a delta, a
/// finished part or a terminal event does; any other type does not, however
/// long, so a padded `response.in_progress` is no progress. An OpenAI chat
/// chunk has no type: one that long is a choice's content.
pub(crate) fn long_event_carries_output(start: &[u8]) -> bool {
    let start = String::from_utf8_lossy(start);
    match first_type(&start) {
        Some(kind) => {
            TERMINAL_TYPES.contains(&kind)
                || LONG_OUTPUT_SUFFIXES
                    .iter()
                    .any(|suffix| kind.ends_with(suffix))
        }
        None => start.contains("\"choices\""),
    }
}

/// The first `"type"` string the start of an event's JSON names: its own
/// type on every wire, which each sends first or close to it.
fn first_type(start: &str) -> Option<&str> {
    let (_, after) = start.split_once("\"type\"")?;
    let value = after.trim_start().strip_prefix(':')?.trim_start();
    value.strip_prefix('"')?.split('"').next()
}

#[cfg(test)]
#[path = "sse_progress_tests.rs"]
mod tests;
