//! The Messages (Anthropic) wire as the attempt transport reads it (#2597):
//! events named by their `event:` line, `message_stop` as the end and an
//! `error` event as a failure whatever its data parsed to, and output in a
//! `content_block_delta`'s text, thinking and partial JSON.
use std::borrow::Cow;

use serde_json::Value;

use crate::domain::inference::value_objects::attempt_diagnostics::TerminalEvent;
use crate::infrastructure::providers::transport::attempt_profile::Surface;
use crate::infrastructure::providers::transport::attempt_transport::attempt_events::named_kind;
use crate::infrastructure::providers::transport::wire_dialect::{Ending, WireDialect, text_len};

/// The Messages (Anthropic) wire.
pub(in crate::infrastructure::providers) struct AnthropicWire;

/// The one Messages dialect every Anthropic profile names.
pub(in crate::infrastructure::providers) static ANTHROPIC_WIRE: AnthropicWire = AnthropicWire;

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
        Some(match name {
            "error" => "error",
            "message_stop" => "message_stop",
            "message_start" => "message_start",
            "message_delta" => "message_delta",
            "content_block_start" => "content_block_start",
            "content_block_delta" => "content_block_delta",
            "content_block_stop" => "content_block_stop",
            "ping" => "ping",
            _ => "",
        })
    }
    fn ends_on_done(&self) -> bool {
        false
    }
    fn event_name<'a>(&self, named: &'a str, _value: &'a Value) -> &'a str {
        named
    }
    fn event_kind(&self, named: &str, _value: &Value) -> &'static str {
        named_kind(named)
    }
    fn output_bytes(&self, value: &Value) -> u64 {
        MESSAGES_DELTA_FIELDS
            .iter()
            .map(|field| text_len(&value["delta"][*field]))
            .sum()
    }
    /// Both Anthropic parsers dispatch by event name and substitute a null
    /// value for malformed JSON. Terminal dispatch must not depend on
    /// successful decoding, or later ignored bytes become feedback.
    fn ending<'a>(&self, named: &str, parsed: Option<&'a Value>) -> Ending<'a> {
        match named {
            "error" => Ending::Failed {
                feedback: parsed.map_or(Cow::Owned(Value::Null), Cow::Borrowed),
                throttle: None,
                record: Some(TerminalEvent::Error),
            },
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
