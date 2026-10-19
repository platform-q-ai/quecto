//! What one vendor's wire tells the attempt transport (#2597): the transport
//! sends, bounds and observes every attempt the same way, and asks the
//! provider's dialect only what its wire means — the words of its errors,
//! how its events are named, which it knows, what each one carries, and how
//! a stream ends. Each vendor implements this once, beside its codec, and
//! knows only its own events; nothing in `transport/` names a vendor.
use std::borrow::Cow;

use serde_json::Value;

use super::attempt_events::Vocabulary;
use super::attempt_profile::Surface;
use crate::domain::inference::services::provider_error::is_billing_error_name;
use crate::domain::inference::value_objects::attempt_diagnostics::{
    ErrorCode, IncompleteReason, TerminalEvent,
};

/// One error type or code a wire names (#2609 review): what an attempt's
/// diagnostics record it as, and what it tells admission and retry.
#[derive(Debug, Clone, Copy)]
pub(in crate::infrastructure::providers) struct ErrorName {
    pub name: &'static str,
    pub code: ErrorCode,
    /// A throttle admission should hear of.
    pub throttle: bool,
    /// A failure no retry fixes: an HTTP error naming it never throttles.
    pub terminal: bool,
}

impl ErrorName {
    /// An error that is neither a throttle nor terminal.
    pub(in crate::infrastructure::providers) const fn plain(
        name: &'static str,
        code: ErrorCode,
    ) -> Self {
        Self {
            name,
            code,
            throttle: false,
            terminal: false,
        }
    }
    /// A throttle.
    pub(in crate::infrastructure::providers) const fn throttle(
        name: &'static str,
        code: ErrorCode,
    ) -> Self {
        Self {
            throttle: true,
            ..Self::plain(name, code)
        }
    }
    /// A failure no retry fixes.
    pub(in crate::infrastructure::providers) const fn terminal(
        name: &'static str,
        code: ErrorCode,
    ) -> Self {
        Self {
            terminal: true,
            ..Self::plain(name, code)
        }
    }
}

/// What one stream event shows the model generated, as the attempt's
/// diagnostics record it: once any event shows one, it stays shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::infrastructure::providers) struct Generated {
    /// Visible text.
    pub text: bool,
    /// A tool call.
    pub tool_call: bool,
    /// Reasoning or thinking the wire streams.
    pub thinking: bool,
}

/// How one stream event ends the attempt, if it does.
pub(in crate::infrastructure::providers) enum Ending<'a> {
    /// The stream goes on.
    Open,
    /// The attempt failed. `feedback` is the typed error body admission
    /// hears; `throttle` is whether the failure is a throttle (at least
    /// whenever the body's typed fields say so,
    /// [`WireDialect::is_throttle`]); `record` is the terminal
    /// event recorded however the data parsed.
    Failed {
        feedback: Cow<'a, Value>,
        throttle: bool,
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
    /// The events this wire names: its own, and no other wire's.
    fn vocabulary(&self) -> &'static Vocabulary;
    /// The type an attempt's event counts file `value` under: by default
    /// its name, when this wire knows it.
    fn event_kind(&self, named: &str, value: &Value) -> &'static str {
        self.vocabulary().kind(self.event_name(named, value))
    }
    /// What the event `value` shows the model generated.
    fn generated(&self, value: &Value) -> Generated;
    /// The bytes of generated output `value` carries.
    fn output_bytes(&self, value: &Value) -> u64;
    /// Whether `value` is an untyped mid-stream error chunk of this wire.
    fn is_error_chunk(&self, _value: &Value) -> bool {
        false
    }
    /// The error types and codes this wire names (#2609 review).
    fn error_names(&self) -> &'static [ErrorName];
    /// Where a payload of this wire keeps its typed error fields (`type`
    /// and `code`): its error objects, in the order they are read.
    fn error_objects<'a>(&self, value: &'a Value) -> Vec<&'a Value>;
    /// Why a reply of this wire stopped short, as `value` says, if it does.
    fn incomplete_reason(&self, _value: &Value) -> Option<IncompleteReason> {
        None
    }
    /// The typed error names of `value`, each field of each error object
    /// in order; a billing name (the domain's list) is read as `None`.
    fn error_fields<'a>(&self, value: &'a Value) -> Vec<Option<&'a str>> {
        self.error_objects(value)
            .into_iter()
            .flat_map(|error| ["type", "code"].map(|field| error[field].as_str()))
            .flatten()
            .map(|name| (!is_billing_error_name(name)).then_some(name))
            .collect()
    }
    /// The [`ErrorName`] this wire knows `name` by.
    fn error_name(&self, name: &str) -> Option<&'static ErrorName> {
        self.error_names().iter().find(|known| known.name == name)
    }
    /// Whether `value`'s typed fields make it a throttle: a throttle name
    /// in its error objects, and no billing name, which takes precedence.
    fn is_throttle(&self, value: &Value) -> bool {
        let fields = self.error_fields(value);
        fields.iter().all(Option::is_some)
            && fields
                .iter()
                .flatten()
                .any(|name| self.error_name(name).is_some_and(|known| known.throttle))
    }
    /// Whether an HTTP error body `value` names a failure no retry fixes: a
    /// billing name, or a terminal name of this wire.
    fn is_terminal_error(&self, value: &Value) -> bool {
        self.error_fields(value).iter().any(|name| match name {
            None => true,
            Some(name) => self.error_name(name).is_some_and(|known| known.terminal),
        })
    }
    /// The terminal event the event `name` (`value`) records, if it is
    /// one: an error chunk of this wire, or a terminal name it knows.
    fn terminal_event(&self, name: &str, value: &Value) -> Option<TerminalEvent> {
        match self.is_error_chunk(value) {
            true => Some(TerminalEvent::Error),
            false => self.vocabulary().terminal(name),
        }
    }
    /// Whether the non-terminal event `name` (`value`) is one this wire
    /// knows.
    fn known_event(&self, name: &str, _value: &Value) -> bool {
        self.vocabulary().known.contains(&name)
    }
    /// Whether the event `value` carries output, restarting the stream
    /// progress bound (#2433, [`super::sse_progress`]): a generated delta, a
    /// finished part that carries content, or the reply's end. An empty
    /// string or list is no output.
    fn carries_output(&self, value: &Value) -> bool;
    /// Whether an event too long to read whole carries output, judged by
    /// the kept `start` of its data, read up to its last whole character
    /// (#2433 review).
    fn long_event_carries_output(&self, start: &str) -> bool;
    /// How the event `parsed` (`None` when its data was not JSON), named
    /// `named` by its `event:` line, ends the attempt.
    fn ending<'a>(&self, named: &str, parsed: Option<&'a Value>) -> Ending<'a>;
}

impl std::fmt::Debug for dyn WireDialect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
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

/// A wire of the transport's tests of its own mechanics (#2597): bounds,
/// framing and diagnostics read alike whatever the vendor. Its events are
/// typed: an `output` event carries output, an `end` event ends the reply,
/// and any other carries none.
#[cfg(test)]
pub(in crate::infrastructure::providers) struct TestWire;

/// The test wire's error names: one of each kind.
#[cfg(test)]
static TEST_ERRORS: [ErrorName; 4] = [
    ErrorName::throttle("throttled", ErrorCode::RateLimitExceeded),
    ErrorName::throttle("busy", ErrorCode::Overloaded),
    ErrorName::terminal("refused", ErrorCode::InvalidRequestError),
    ErrorName::plain("broken", ErrorCode::ServerError),
];

#[cfg(test)]
static TEST_VOCABULARY: Vocabulary = Vocabulary {
    known: &["output", "working"],
    terminal: &[("end", TerminalEvent::Done)],
};

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
    fn vocabulary(&self) -> &'static Vocabulary {
        &TEST_VOCABULARY
    }
    fn error_names(&self) -> &'static [ErrorName] {
        &TEST_ERRORS
    }
    fn error_objects<'a>(&self, value: &'a Value) -> Vec<&'a Value> {
        value.get("error").into_iter().collect()
    }
    fn incomplete_reason(&self, value: &Value) -> Option<IncompleteReason> {
        (value["stopped"] == "max_output_tokens").then_some(IncompleteReason::MaxOutputTokens)
    }
    fn generated(&self, value: &Value) -> Generated {
        Generated {
            text: value["type"] == "output",
            ..Generated::default()
        }
    }
    fn output_bytes(&self, value: &Value) -> u64 {
        match value["type"] == "output" {
            true => text_len(&value["text"]),
            false => 0,
        }
    }
    fn carries_output(&self, value: &Value) -> bool {
        matches!(value["type"].as_str(), Some("output" | "end"))
    }
    fn long_event_carries_output(&self, start: &str) -> bool {
        matches!(
            super::sse_progress::first_type(start),
            Some("output" | "end")
        )
    }
    fn ending<'a>(&self, _named: &str, _parsed: Option<&'a Value>) -> Ending<'a> {
        Ending::Open
    }
}
