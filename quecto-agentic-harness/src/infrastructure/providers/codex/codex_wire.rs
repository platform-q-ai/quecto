//! The Responses (Codex) wire as the attempt transport reads it (#2597):
//! typed events, `[DONE]` or `response.completed` as the end,
//! `response.failed`, `error`, an untyped error chunk (#2249 review) and
//! `response.incomplete` — unless cut off at the output limit (#2593) — as
//! a failure, and output in each delta event's `delta` string. It knows
//! only the Responses events.
use std::borrow::Cow;

use serde_json::Value;

use crate::domain::inference::value_objects::attempt_diagnostics::{
    ErrorCode, IncompleteReason, TerminalEvent,
};
use crate::infrastructure::providers::openai::openai_stream_error::is_stream_error_chunk;
use crate::infrastructure::providers::transport::attempt_events::Vocabulary;
use crate::infrastructure::providers::transport::sse_progress::{filled, first_type};
use crate::infrastructure::providers::transport::wire_dialect::{
    Ending, ErrorName, Generated, WireDialect, text_len,
};

/// The Responses (Codex) wire.
pub(in crate::infrastructure::providers) struct CodexWire;

/// The one Responses dialect every Codex profile names.
pub(in crate::infrastructure::providers) static CODEX_WIRE: CodexWire = CodexWire;

/// The Responses events (#2158): every non-terminal event the Codex stream
/// handler consumes or the wire sends as ordinary traffic, and its terminal
/// events.
static RESPONSES: Vocabulary = Vocabulary {
    known: &[
        "response.created",
        "response.in_progress",
        "response.output_item.added",
        "response.output_item.done",
        "response.content_part.added",
        "response.content_part.done",
        "response.output_text.delta",
        "response.output_text.done",
        "response.function_call_arguments.delta",
        "response.function_call_arguments.done",
        "response.refusal.delta",
        "response.refusal.done",
        "response.reasoning_summary_part.added",
        "response.reasoning_summary_part.done",
        "response.reasoning_summary_text.delta",
        "response.reasoning_summary_text.done",
        "response.reasoning.summary_text.delta",
    ],
    terminal: &[
        ("response.completed", TerminalEvent::ResponseCompleted),
        ("response.failed", TerminalEvent::ResponseFailed),
        ("response.incomplete", TerminalEvent::ResponseIncomplete),
        ("error", TerminalEvent::Error),
    ],
};

/// The error types and codes the Responses wire names (#2609 review),
/// with the throttles the official Codex client reads as a rate limit
/// (`slow_down`) and as overloaded (`server_is_overloaded`; #2620 review).
static RESPONSES_ERRORS: [ErrorName; 8] = [
    ErrorName::throttle("rate_limit_exceeded", ErrorCode::RateLimitExceeded),
    ErrorName::throttle("slow_down", ErrorCode::RateLimitExceeded),
    ErrorName::throttle("server_is_overloaded", ErrorCode::Overloaded),
    ErrorName::plain("server_error", ErrorCode::ServerError),
    ErrorName::plain("insufficient_quota", ErrorCode::InsufficientQuota),
    ErrorName::plain("usage_limit_reached", ErrorCode::UsageLimitReached),
    ErrorName::terminal("invalid_request_error", ErrorCode::InvalidRequestError),
    ErrorName::terminal("invalid_api_key", ErrorCode::InvalidApiKey),
];

/// The reasoning summary deltas: generated thinking (#2151).
const REASONING_DELTAS: &[&str] = &[
    "response.reasoning_summary_text.delta",
    "response.reasoning.summary_text.delta",
];

/// The type suffixes of Responses events that carry output when long:
/// deltas and finished parts (#2433 review).
const LONG_OUTPUT_SUFFIXES: &[&str] = &[".delta", ".done"];

/// Responses API (Codex) events whose `delta` string is generated output.
/// Each is also a known event (#2158).
const RESPONSES_OUTPUT_DELTAS: &[&str] = &[
    "response.output_text.delta",
    "response.function_call_arguments.delta",
    "response.refusal.delta",
    "response.reasoning_summary_text.delta",
    "response.reasoning.summary_text.delta",
];

impl WireDialect for CodexWire {
    fn name(&self) -> &'static str {
        "Codex"
    }
    fn send_error(&self, error: &reqwest::Error) -> String {
        format!(
            "Codex request failed: {}",
            crate::infrastructure::providers::transport::sse_common::format_send_error(error)
        )
    }
    fn retry_after(&self, _headers: &reqwest::header::HeaderMap) -> String {
        String::new()
    }
    fn vocabulary(&self) -> &'static Vocabulary {
        &RESPONSES
    }
    fn generated(&self, value: &Value) -> Generated {
        let kind = value["type"].as_str();
        let delta = value["delta"].as_str().is_some_and(|text| !text.is_empty());
        Generated {
            text: kind == Some("response.output_text.delta") && delta,
            tool_call: value["item"]["type"] == "function_call",
            thinking: kind.is_some_and(|kind| REASONING_DELTAS.contains(&kind)) && delta,
        }
    }
    fn output_bytes(&self, value: &Value) -> u64 {
        match value["type"].as_str() {
            Some(kind) if RESPONSES_OUTPUT_DELTAS.contains(&kind) => text_len(&value["delta"]),
            _ => 0,
        }
    }
    fn is_error_chunk(&self, value: &Value) -> bool {
        is_untyped_error_chunk(value)
    }
    fn error_names(&self) -> &'static [ErrorName] {
        &RESPONSES_ERRORS
    }
    /// An `error` event's own fields (`{"type":"error","code":…}`), its
    /// `error` object, and a failed response's `response.error`.
    fn error_objects<'a>(&self, value: &'a Value) -> Vec<&'a Value> {
        let event = (value["type"] == "error").then_some(value);
        [event, value.get("error"), value["response"].get("error")]
            .into_iter()
            .flatten()
            .collect()
    }
    /// A `response.incomplete`'s `incomplete_details.reason`.
    fn incomplete_reason(&self, value: &Value) -> Option<IncompleteReason> {
        match value["response"]["incomplete_details"]["reason"].as_str() {
            Some("max_output_tokens") => Some(IncompleteReason::MaxOutputTokens),
            Some("content_filter") => Some(IncompleteReason::ContentFilter),
            _ => None,
        }
    }
    /// A Responses event carries output when it ends the reply, or holds a
    /// generated `delta` string (text, tool-call arguments, refusal,
    /// reasoning or reasoning summary), or a finished part with content: a
    /// `text`, `arguments` or `refusal`, an output `item` holding arguments
    /// or a content or summary part with text, or a content `part` with
    /// text. A tool call just opened is none yet.
    fn carries_output(&self, value: &Value) -> bool {
        let item = &value["item"];
        value["type"]
            .as_str()
            .is_some_and(|kind| RESPONSES.terminal(kind).is_some())
            || ["delta", "text", "arguments", "refusal"]
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
    /// A Responses event names its own type first: a delta, a finished part
    /// or a terminal event carries output; any other type does not, however
    /// long, so a padded `response.in_progress` is no progress.
    fn long_event_carries_output(&self, start: &str) -> bool {
        first_type(start).is_some_and(|kind| {
            RESPONSES.terminal(kind).is_some()
                || LONG_OUTPUT_SUFFIXES
                    .iter()
                    .any(|suffix| kind.ends_with(suffix))
        })
    }
    fn ending<'a>(&self, _named: &str, parsed: Option<&'a Value>) -> Ending<'a> {
        let Some(value) = parsed else {
            return Ending::Open;
        };
        // A reply that finishes — completed, or cut off at its output
        // limit (#2593) — completes; any other terminal event fails.
        if finishes_reply(value) {
            Ending::Completed { record: None }
        } else if matches!(
            value["type"].as_str(),
            Some("response.failed" | "response.incomplete" | "error")
        ) || is_untyped_error_chunk(value)
        {
            Ending::Failed {
                feedback: Cow::Borrowed(value),
                throttle: self.is_throttle(value),
                record: None,
            }
        } else {
            Ending::Open
        }
    }
}

/// Whether a Responses event ends its reply at the output limit (#2593):
/// `response.incomplete` with a reason the domain allowlists. Such a reply
/// is cut off, not failed: it finishes with `MaxTokens` and what it got.
pub(in crate::infrastructure::providers) fn ends_at_output_limit(event: &Value) -> bool {
    event["type"].as_str() == Some("response.incomplete")
        && event["response"]["incomplete_details"]["reason"]
            .as_str()
            .is_some_and(
                crate::domain::inference::services::output_limit::is_responses_output_limit,
            )
}

/// Whether a Responses event finishes its reply: `response.completed`, or
/// a reply cut off at the output limit ([`ends_at_output_limit`]).
pub(in crate::infrastructure::providers) fn finishes_reply(event: &Value) -> bool {
    event["type"].as_str() == Some("response.completed") || ends_at_output_limit(event)
}

/// A content or summary part holding text or a refusal.
fn part(part: &Value) -> bool {
    filled(&part["text"]) || filled(&part["refusal"])
}

/// Whether a Responses (Codex) SSE chunk is an untyped error chunk
/// (#2249 review): it has no string `type` (none, or `null`) and is an
/// error chunk by [`is_stream_error_chunk`] — a gateway's or a compatible
/// server's shape. A typed event keeps the meaning its type gives it.
pub(in crate::infrastructure::providers) fn is_untyped_error_chunk(
    value: &serde_json::Value,
) -> bool {
    value["type"].as_str().is_none() && is_stream_error_chunk(value)
}
#[cfg(test)]
#[path = "codex_wire_tests.rs"]
mod tests;
