//! The OpenAI chat wire as the attempt transport reads it (#2597): chunks
//! with no event names, `[DONE]` as the end, output in the first choice's
//! delta, and an error chunk (#2155, #2236) as a failure that throttles
//! admission by its typed fields or a numeric 429/529 code.
use std::borrow::Cow;

use serde_json::Value;

use super::openai_stream_error::{is_stream_error_chunk, is_throttle_chunk};
use crate::infrastructure::providers::transport::admission_feedback::is_typed_throttle;
use crate::infrastructure::providers::transport::attempt_events::KNOWN_EVENTS;
use crate::infrastructure::providers::transport::attempt_profile::Surface;
use crate::infrastructure::providers::transport::wire_dialect::{Ending, WireDialect, text_len};

/// The OpenAI chat wire.
pub(in crate::infrastructure::providers) struct OpenAiWire;

/// The one OpenAI chat dialect every OpenAI profile names.
pub(in crate::infrastructure::providers) static OPENAI_WIRE: OpenAiWire = OpenAiWire;

/// OpenAI chat `choices[].delta` fields that hold generated output. The
/// reasoning is one of [`CHAT_REASONING_FIELDS`], counted apart.
const CHAT_DELTA_FIELDS: &[&str] = &["content", "refusal"];

/// The names OpenAI-compatible providers stream reasoning under: one or the
/// other, never both. As the handler reads it (`openai_sse.rs`), the first
/// present is the reasoning (#2210 review).
const CHAT_REASONING_FIELDS: [&str; 2] = ["reasoning", "reasoning_content"];

impl WireDialect for OpenAiWire {
    fn name(&self) -> &'static str {
        "OpenAI"
    }
    fn send_error(&self, error: &reqwest::Error) -> String {
        format!(
            "HTTP error: {}",
            crate::infrastructure::providers::transport::sse_common::format_send_error(error)
        )
    }
    fn strict_error_body(&self, surface: Surface) -> bool {
        matches!(surface, Surface::Chat)
    }
    fn event_kind(&self, _named: &str, value: &Value) -> &'static str {
        chat_kind(value)
    }
    fn output_bytes(&self, value: &Value) -> u64 {
        chat_output(value)
    }
    fn is_error_chunk(&self, value: &Value) -> bool {
        is_stream_error_chunk(value)
    }
    fn known_event(&self, name: &str, value: &Value) -> bool {
        KNOWN_EVENTS.contains(&name) || value.get("choices").is_some()
    }
    fn ending<'a>(&self, _named: &str, parsed: Option<&'a Value>) -> Ending<'a> {
        match parsed {
            // A numeric 429/529 chunk throttles too (#2155 review).
            Some(value) if is_stream_error_chunk(value) => Ending::Failed {
                feedback: Cow::Borrowed(value),
                throttle: is_typed_throttle(value) || is_throttle_chunk(value),
                record: None,
            },
            _ => Ending::Open,
        }
    }
}

/// An OpenAI chat chunk's type: the first delta field of its first choice
/// that carries something, else its finish, else `chat.empty`.
fn chat_kind(value: &serde_json::Value) -> &'static str {
    let choice = &value["choices"][0];
    let delta = &choice["delta"];
    let carries = |field: &str| match &delta[field] {
        serde_json::Value::String(text) => !text.is_empty(),
        serde_json::Value::Array(items) => !items.is_empty(),
        _ => false,
    };
    [
        ("tool_calls", "chat.delta.tool_calls"),
        ("content", "chat.delta.content"),
        ("reasoning", "chat.delta.reasoning"),
        ("reasoning_content", "chat.delta.reasoning"),
        ("refusal", "chat.delta.refusal"),
    ]
    .into_iter()
    .find(|(field, _)| carries(field))
    .map(|(_, kind)| kind)
    .unwrap_or(match choice["finish_reason"].is_string() {
        true => "chat.finish",
        false => "chat.empty",
    })
}
/// The bytes of output an OpenAI chat chunk carries: every choice's
/// content, refusal, reasoning and tool-call arguments (#2210).
fn chat_output(value: &Value) -> u64 {
    let Some(choices) = value["choices"].as_array() else {
        return 0;
    };
    choices
        .iter()
        .map(|choice| {
            let delta = &choice["delta"];
            let [reasoning, alternative] = CHAT_REASONING_FIELDS;
            let reasoning = delta
                .get(reasoning)
                .or_else(|| delta.get(alternative))
                .map_or(0, text_len);
            let fields: u64 = CHAT_DELTA_FIELDS
                .iter()
                .map(|field| text_len(&delta[*field]))
                .sum::<u64>()
                .saturating_add(reasoning);
            let arguments: u64 = delta["tool_calls"].as_array().map_or(0, |calls| {
                calls
                    .iter()
                    .map(|call| text_len(&call["function"]["arguments"]))
                    .sum()
            });
            fields.saturating_add(arguments)
        })
        .fold(0, u64::saturating_add)
}

#[cfg(test)]
#[path = "openai_wire_tests.rs"]
mod tests;
