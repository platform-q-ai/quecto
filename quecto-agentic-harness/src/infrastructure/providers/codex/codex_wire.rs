//! The Responses (Codex) wire as the attempt transport reads it (#2597):
//! typed events, `[DONE]` or `response.completed` as the end,
//! `response.failed`, `response.incomplete`, `error` and an untyped error
//! chunk (#2249 review) as a failure, and output in each delta event's
//! `delta` string.
use std::borrow::Cow;

use serde_json::Value;

use crate::infrastructure::providers::openai::openai_stream_error::is_stream_error_chunk;
use crate::infrastructure::providers::transport::attempt_transport::attempt_events::named_kind;
use crate::infrastructure::providers::transport::wire_dialect::{Ending, WireDialect, text_len};

/// The Responses (Codex) wire.
pub(in crate::infrastructure::providers) struct CodexWire;

/// The one Responses dialect every Codex profile names.
pub(in crate::infrastructure::providers) static CODEX_WIRE: CodexWire = CodexWire;

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
    fn event_kind(&self, _named: &str, value: &Value) -> &'static str {
        named_kind(value["type"].as_str().unwrap_or_default())
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
                throttle: None,
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
