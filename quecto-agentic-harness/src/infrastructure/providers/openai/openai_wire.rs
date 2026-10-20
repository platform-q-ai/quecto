//! The OpenAI chat wire as the attempt transport reads it (#2597): chunks
//! with no event names, `[DONE]` as the end, output in the first choice's
//! delta, and an error chunk (#2155, #2236) as a failure that throttles
//! admission by its typed fields or a numeric 429/529 code. Its chunks are
//! named by their choices: it knows no other wire's events.
use std::borrow::Cow;

use serde_json::Value;

use super::openai_stream_error::{is_stream_error_chunk, is_throttle_chunk};
use crate::domain::inference::value_objects::attempt_diagnostics::ErrorCode;
use crate::infrastructure::providers::transport::attempt_events::Vocabulary;
use crate::infrastructure::providers::transport::attempt_profile::Surface;
use crate::infrastructure::providers::transport::sse_progress::filled;
use crate::infrastructure::providers::transport::wire_dialect::{
    Ending, ErrorName, Generated, WireDialect, text_len,
};

/// The error types and codes the OpenAI-compatible chat wire names (#2609
/// review): OpenAI's and those of the compatible vendors that serve it
/// (OpenRouter relays an upstream's, such as an overloaded Claude's), as
/// the chunk renderer ([`super::openai_stream_error`]) knows them. Read
/// only from a chunk's or a body's `error` object.
static CHAT_ERRORS: [ErrorName; 10] = [
    ErrorName::throttle("rate_limit_exceeded", ErrorCode::RateLimitExceeded),
    ErrorName::throttle("rate_limit_error", ErrorCode::RateLimitExceeded),
    ErrorName::throttle("overloaded_error", ErrorCode::Overloaded),
    ErrorName::plain("server_error", ErrorCode::ServerError),
    ErrorName::plain("insufficient_quota", ErrorCode::InsufficientQuota),
    ErrorName::plain("usage_limit_reached", ErrorCode::UsageLimitReached),
    ErrorName::terminal("authentication_error", ErrorCode::AuthenticationError),
    ErrorName::terminal("permission_error", ErrorCode::PermissionError),
    ErrorName::terminal("invalid_request_error", ErrorCode::InvalidRequestError),
    ErrorName::terminal("invalid_api_key", ErrorCode::InvalidApiKey),
];

/// The chat wire names no events: a chunk is known by its choices, and an
/// error chunk is its only terminal event.
static CHAT: Vocabulary = Vocabulary {
    known: &[],
    terminal: &[],
};

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
    fn vocabulary(&self) -> &'static Vocabulary {
        &CHAT
    }
    fn error_names(&self) -> &'static [ErrorName] {
        &CHAT_ERRORS
    }
    /// A chunk's or a body's `error` object.
    fn error_objects<'a>(&self, value: &'a Value) -> Vec<&'a Value> {
        value.get("error").into_iter().collect()
    }
    fn event_kind(&self, _named: &str, value: &Value) -> &'static str {
        chat_kind(value)
    }
    /// What the first choice's delta carries. OpenAI-compatible reasoning
    /// models (DeepSeek, GLM, OpenRouter) stream thinking as `reasoning` or
    /// `reasoning_content` (#2151).
    fn generated(&self, value: &Value) -> Generated {
        let delta = &value["choices"][0]["delta"];
        let text = |field: &str| delta[field].as_str().is_some_and(|s| !s.is_empty());
        Generated {
            text: text("content"),
            tool_call: delta["tool_calls"]
                .as_array()
                .is_some_and(|calls| !calls.is_empty()),
            thinking: CHAT_REASONING_FIELDS.iter().any(|field| text(field)),
        }
    }
    fn output_bytes(&self, value: &Value) -> u64 {
        chat_output(value)
    }
    fn is_error_chunk(&self, value: &Value) -> bool {
        is_stream_error_chunk(value)
    }
    fn known_event(&self, _name: &str, value: &Value) -> bool {
        value.get("choices").is_some()
    }
    /// A chat chunk carries output when a choice's delta holds content,
    /// reasoning, a refusal or a tool call naming a function or carrying
    /// arguments, or the choice finishes.
    fn carries_output(&self, value: &Value) -> bool {
        value["choices"]
            .as_array()
            .is_some_and(|choices| choices.iter().any(chat_choice))
    }
    /// A chat chunk opens with its `choices` (a `"type"` inside them, such
    /// as a tool call's, is not the chunk's): one too long to read whole
    /// carries output when its choices hold content, reasoning, a refusal or
    /// a tool call (#2433 review round 3).
    fn long_event_carries_output(&self, start: &str) -> bool {
        let chat = match (start.find("\"choices\""), start.find("\"type\"")) {
            (Some(choices), Some(kind)) => choices < kind,
            (Some(_), None) => true,
            (None, _) => false,
        };
        chat && CHAT_OUTPUT_FIELDS.iter().any(|field| start.contains(field))
    }
    fn ending<'a>(&self, _named: &str, parsed: Option<&'a Value>) -> Ending<'a> {
        match parsed {
            // A numeric 429/529 chunk throttles too (#2155 review).
            Some(value) if is_stream_error_chunk(value) => Ending::Failed {
                feedback: Cow::Borrowed(value),
                throttle: self.is_throttle(value) || is_throttle_chunk(value),
                record: None,
            },
            _ => Ending::Open,
        }
    }
}

/// The fields of a chat choice's delta that hold output, as a long chunk's
/// start names them.
const CHAT_OUTPUT_FIELDS: &[&str] = &[
    "\"content\"",
    "\"reasoning\"",
    "\"reasoning_content\"",
    "\"refusal\"",
    "\"tool_calls\"",
];

/// A chat choice that carries output.
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
