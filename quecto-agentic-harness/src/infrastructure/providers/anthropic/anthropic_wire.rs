//! The Messages (Anthropic) wire as the attempt transport reads it (#2597):
//! events named by their `event:` line, `message_stop` as the end and an
//! `error` event as a failure whatever its data parsed to, and output in a
//! `content_block_delta`'s text, thinking and partial JSON. It knows only
//! the Messages events.
use std::borrow::Cow;

use serde_json::Value;

use crate::domain::inference::value_objects::attempt_diagnostics::{ErrorCode, TerminalEvent};
use crate::infrastructure::providers::transport::attempt_events::Vocabulary;
use crate::infrastructure::providers::transport::attempt_profile::Surface;
use crate::infrastructure::providers::transport::sse_progress::{filled, first_type};
use crate::infrastructure::providers::transport::wire_dialect::{
    Ending, ErrorName, Generated, WireDialect, text_len,
};

/// The Messages (Anthropic) wire.
pub(in crate::infrastructure::providers) struct AnthropicWire;

/// The one Messages dialect every Anthropic profile names.
pub(in crate::infrastructure::providers) static ANTHROPIC_WIRE: AnthropicWire = AnthropicWire;

/// The Messages events (#2158): every non-terminal event the Anthropic
/// stream handlers consume or the wire sends as ordinary traffic (`ping`),
/// and its terminal events.
static MESSAGES: Vocabulary = Vocabulary {
    known: &[
        "message_start",
        "message_delta",
        "content_block_start",
        "content_block_delta",
        "content_block_stop",
        "ping",
    ],
    terminal: &[
        ("error", TerminalEvent::Error),
        ("message_stop", TerminalEvent::MessageStop),
    ],
};

/// The error types the Messages wire names (#2609 review).
static MESSAGES_ERRORS: [ErrorName; 5] = [
    ErrorName::throttle("rate_limit_error", ErrorCode::RateLimitExceeded),
    ErrorName::throttle("overloaded_error", ErrorCode::Overloaded),
    ErrorName::terminal("authentication_error", ErrorCode::AuthenticationError),
    ErrorName::terminal("permission_error", ErrorCode::PermissionError),
    ErrorName::terminal("invalid_request_error", ErrorCode::InvalidRequestError),
];

/// Anthropic `content_block_delta` fields that hold generated output:
/// `text_delta`, `thinking_delta` and `input_json_delta`.
const MESSAGES_DELTA_FIELDS: &[&str] = &["text", "thinking", "partial_json"];

impl WireDialect for AnthropicWire {
    fn name(&self) -> &'static str {
        "Anthropic"
    }
    fn send_error(&self, error: &reqwest::Error) -> String {
        format!("HTTP error: {error}")
    }
    fn read_label(&self, surface: Surface) -> &'static str {
        match surface {
            Surface::Assembled => "stream",
            Surface::Chat | Surface::Incremental => "response",
        }
    }
    fn strict_error_body(&self, surface: Surface) -> bool {
        matches!(surface, Surface::Chat)
    }
    fn event_line(&self, name: &str) -> Option<&'static str> {
        Some(MESSAGES.name(name).unwrap_or(""))
    }
    fn ends_on_done(&self) -> bool {
        false
    }
    fn event_name<'a>(&self, named: &'a str, _value: &'a Value) -> &'a str {
        named
    }
    fn vocabulary(&self) -> &'static Vocabulary {
        &MESSAGES
    }
    fn error_names(&self) -> &'static [ErrorName] {
        &MESSAGES_ERRORS
    }
    /// The `error` object of an `error` event or an error body
    /// (`{"type":"error","error":{"type":…}}`).
    fn error_objects<'a>(&self, value: &'a Value) -> Vec<&'a Value> {
        value.get("error").into_iter().collect()
    }
    fn generated(&self, value: &Value) -> Generated {
        let delta = |field: &str| {
            value["delta"][field]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        };
        Generated {
            text: delta("text"),
            tool_call: value["content_block"]["type"] == "tool_use",
            thinking: delta("thinking"),
        }
    }
    fn output_bytes(&self, value: &Value) -> u64 {
        MESSAGES_DELTA_FIELDS
            .iter()
            .map(|field| text_len(&value["delta"][*field]))
            .sum()
    }
    /// A Messages event carries output when it ends the reply, or holds a
    /// delta with `text`, `thinking` or `partial_json`, or a `stop_reason`,
    /// or a `content_block` holding text or thinking. A tool call just
    /// opened is none yet.
    fn carries_output(&self, value: &Value) -> bool {
        let delta = &value["delta"];
        let block = &value["content_block"];
        value["type"]
            .as_str()
            .is_some_and(|kind| MESSAGES.terminal(kind).is_some())
            || MESSAGES_DELTA_FIELDS
                .iter()
                .any(|field| filled(&delta[*field]))
            || delta["stop_reason"].is_string()
            || ["text", "thinking"]
                .iter()
                .any(|field| filled(&block[*field]))
    }
    /// A Messages event names its own type first (or its delta's): a delta
    /// or a terminal event carries output; any other type does not, however
    /// long.
    fn long_event_carries_output(&self, start: &str) -> bool {
        first_type(start)
            .is_some_and(|kind| MESSAGES.terminal(kind).is_some() || kind.ends_with("_delta"))
    }
    /// Both Anthropic parsers dispatch by event name and substitute a null
    /// value for malformed JSON. Terminal dispatch must not depend on
    /// successful decoding, or later ignored bytes become feedback.
    fn ending<'a>(&self, named: &str, parsed: Option<&'a Value>) -> Ending<'a> {
        match named {
            "error" => {
                let feedback = parsed.map_or(Cow::Owned(Value::Null), Cow::Borrowed);
                Ending::Failed {
                    throttle: self.is_throttle(&feedback),
                    feedback,
                    record: Some(TerminalEvent::Error),
                }
            }
            "message_stop" => Ending::Completed {
                record: Some(TerminalEvent::MessageStop),
            },
            _ => Ending::Open,
        }
    }
}

#[cfg(test)]
#[path = "anthropic_wire_tests.rs"]
mod tests;
