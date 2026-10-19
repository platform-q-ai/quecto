//! What one vendor's wire tells the attempt transport (#2597): the transport
//! sends, bounds and observes every attempt the same way, and asks the
//! provider's dialect only what its wire means — the words of its errors,
//! how its events are named, what each one carries, and how a stream ends.
//! Each vendor implements this once, beside its codec; nothing in
//! `transport/` names a vendor.
use std::borrow::Cow;

use serde_json::Value;

use super::attempt_profile::Surface;
use crate::domain::inference::value_objects::attempt_diagnostics::TerminalEvent;

/// How one stream event ends the attempt, if it does.
pub(in crate::infrastructure::providers) enum Ending<'a> {
    /// The stream goes on.
    Open,
    /// The attempt failed. `feedback` is the typed error body admission
    /// hears; `throttle` is whether it is a throttle when the wire knows a
    /// throttle by more than its typed fields (`None`: by those alone);
    /// `record` is the terminal event recorded however the data parsed.
    Failed {
        feedback: Cow<'a, Value>,
        throttle: Option<bool>,
        record: Option<TerminalEvent>,
    },
    /// The reply ended; `record` as for [`Ending::Failed`].
    Completed { record: Option<TerminalEvent> },
}

/// One vendor's wire, as the attempt transport reads it.
pub(in crate::infrastructure::providers) trait WireDialect:
    Sync
{
    /// The vendor's name in the transport's error messages.
    fn name(&self) -> &'static str;
    /// The message of a request that could not be sent.
    fn send_error(&self, error: &reqwest::Error) -> String;
    /// What a failed body read on `surface` calls the body.
    fn read_label(&self, _surface: Surface) -> &'static str {
        "response"
    }
    /// Whether an error body that cannot be read fails the read itself on
    /// `surface`, rather than being read as empty.
    fn strict_error_body(&self, _surface: Surface) -> bool {
        false
    }
    /// The retry-after note an HTTP error's message ends with.
    fn retry_after(&self, headers: &reqwest::header::HeaderMap) -> String {
        super::sse_common::retry_after_suffix(headers)
    }
    /// The name an `event: <name>` line gives the data after it, when this
    /// wire names its events that way: `Some("")` for a name the harness
    /// does not know, `None` when the wire names no events by line.
    fn event_line(&self, _name: &str) -> Option<&'static str> {
        None
    }
    /// Whether a `[DONE]` data line ends the reply.
    fn ends_on_done(&self) -> bool {
        true
    }
    /// The name of the event `value`, given the name its `event:` line
    /// gave it (`named`, empty when none did).
    fn event_name<'a>(&self, _named: &'a str, value: &'a Value) -> &'a str {
        value["type"].as_str().unwrap_or("")
    }
    /// The type an attempt's event counts file `value` under.
    fn event_kind(&self, named: &str, value: &Value) -> &'static str;
    /// The bytes of generated output `value` carries.
    fn output_bytes(&self, value: &Value) -> u64;
    /// Whether `value` is an untyped mid-stream error chunk of this wire.
    fn is_error_chunk(&self, _value: &Value) -> bool {
        false
    }
    /// Whether the non-terminal event `name` (`value`) is one the harness
    /// knows.
    fn known_event(&self, name: &str, _value: &Value) -> bool {
        super::attempt_transport::attempt_events::KNOWN_EVENTS.contains(&name)
    }
    /// How the event `parsed` (`None` when its data was not JSON), named
    /// `named` by its `event:` line, ends the attempt.
    fn ending<'a>(&self, named: &str, parsed: Option<&'a Value>) -> Ending<'a>;
}

/// The bytes of a string field, 0 for any other value.
pub(in crate::infrastructure::providers) fn text_len(value: &Value) -> u64 {
    value.as_str().map_or(0, |text| text.len() as u64)
}

/// A real `reqwest::Error` of the send kind, for the dialects' wording
/// tests: a connection to a closed loopback port is refused
/// deterministically.
#[cfg(test)]
pub(in crate::infrastructure::providers) async fn refused_send() -> reqwest::Error {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap()
        .get("http://127.0.0.1:1/")
        .send()
        .await
        .expect_err("connection to closed port must fail")
}

/// A wire with no vocabulary of its own, for the transport's tests of its
/// own mechanics (#2597): bounds, framing and diagnostics read alike
/// whatever the vendor.
#[cfg(test)]
pub(in crate::infrastructure::providers) struct TestWire;

#[cfg(test)]
pub(in crate::infrastructure::providers) static TEST_WIRE: TestWire = TestWire;

#[cfg(test)]
impl WireDialect for TestWire {
    fn name(&self) -> &'static str {
        "Test"
    }
    fn send_error(&self, error: &reqwest::Error) -> String {
        format!("HTTP error: {error}")
    }
    fn event_kind(&self, _named: &str, _value: &Value) -> &'static str {
        super::attempt_transport::attempt_events::UNKNOWN_KIND
    }
    fn output_bytes(&self, _value: &Value) -> u64 {
        0
    }
    fn ending<'a>(&self, _named: &str, _parsed: Option<&'a Value>) -> Ending<'a> {
        Ending::Open
    }
}
