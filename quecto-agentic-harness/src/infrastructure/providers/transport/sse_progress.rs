//! Which SSE events carry output (#2433).
//!
//! A reply once streamed ~11 recognised events a second for 15 minutes
//! with no output at all: every event restarted the stream idle bound, so
//! nothing ended it short of an outer deadline. The stream progress bound
//! ([`super::stream_idle::EventIdle`]) is restarted only by an event that
//! carries output, which only its wire can tell
//! ([`WireDialect::carries_output`], #2597): a generated delta, a finished
//! part that carries content, or the reply's end. A `[DONE]` line ends the
//! reply of a wire that ends on it.
//!
//! An empty string or list is no output, so a stream of empty deltas makes
//! no progress. Reasoning a model streams is output, so a model that keeps
//! reasoning is never cut short by this bound. One that thinks silently is
//! bounded by the idle bound alone. One that sends only the odd keep-alive
//! event while it thinks restarts the idle bound with each, so it is the
//! progress bound's: its count rule would wait for
//! [`super::stream_idle::PROGRESS_EVENTS`] of them (50 minutes of events
//! every 15 s), and its backstop
//! ([`super::stream_idle::PROGRESS_BACKSTOP_FACTOR`] progress limits) ends
//! it first, at 15 minutes by default.
use serde_json::Value;

use super::wire_dialect::WireDialect;

/// Whether the SSE event whose data is `data` carries output on `wire`.
pub(in crate::infrastructure::providers) fn carries_output(
    wire: &dyn WireDialect,
    data: &str,
) -> bool {
    if wire.ends_on_done() && super::sse_end::is_done_marker(data.trim()) {
        return true;
    }
    serde_json::from_str::<Value>(data).is_ok_and(|value| wire.carries_output(&value))
}

/// Whether an event too long to read whole carries output on `wire`,
/// judged by the kept `start` of its data (#2433 review): the wire judges
/// it by what its start names, so a padded event that carries none is no
/// progress, however long.
pub(in crate::infrastructure::providers) fn long_event_carries_output(
    wire: &dyn WireDialect,
    start: &[u8],
) -> bool {
    wire.long_event_carries_output(within_characters(start))
}

/// A string or list with something in it.
pub(in crate::infrastructure::providers) fn filled(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        _ => false,
    }
}

/// The longest run of whole characters `bytes` starts with: a kept start
/// cut inside a character (or one with an invalid byte) is read up to it,
/// without a copy (#2433 review round 3).
fn within_characters(bytes: &[u8]) -> &str {
    match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            let valid = &bytes[..error.valid_up_to()];
            std::str::from_utf8(valid).expect("valid up to the first invalid byte")
        }
    }
}

/// The first `"type"` string the start of an event's JSON names: its own
/// type on a wire whose events send it first or close to it.
pub(in crate::infrastructure::providers) fn first_type(start: &str) -> Option<&str> {
    let (_, after) = start.split_once("\"type\"")?;
    let value = after.trim_start().strip_prefix(':')?.trim_start();
    value.strip_prefix('"')?.split('"').next()
}

#[cfg(test)]
#[path = "sse_progress_tests.rs"]
mod tests;
