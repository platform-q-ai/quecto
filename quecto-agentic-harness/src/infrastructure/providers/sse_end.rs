//! How an SSE body ends (#2236, #2249 review), shared by every vendor's
//! handlers, parsers, pumps and attempt observers so they agree:
//!
//! - `[DONE]` is recognised with trailing whitespace ([`is_done_marker`]);
//! - a body that ends before its protocol's terminal event is cut short,
//!   worded as the transport cut it is ([`CUT_SHORT`]), and one with no
//!   event at all is an empty stream ([`EMPTY_STREAM`]);
//! - the last line of a body is a line even without a newline
//!   ([`last_line`]);
//! - usage a cut-short reply reported is still counted
//!   ([`UnfinishedReply`]).
use crate::domain::error::DomainError;
use crate::domain::inference::events::request_observation::RequestTrace;
use crate::domain::message::{LlmResponse, UsageInfo};

pub(crate) use crate::domain::inference::services::provider_error::EMPTY_STREAM;

/// What every cut-short error says, after its protocol's name and before
/// the terminal event it never saw: `connection` makes the classifier read
/// it as a retryable network failure, as the transport cut it is.
pub(crate) const CUT_SHORT: &str = "ended without completion: connection closed before";

/// Whether `data` (an SSE `data:` payload) is the `[DONE]` marker; trailing
/// whitespace is allowed, as some servers send it.
pub(crate) fn is_done_marker(data: &str) -> bool {
    data.trim_end() == "[DONE]"
}

/// Whether `message` says a body ended before its terminal event: cut
/// short after events, or an empty stream with none. An allowlist of the
/// two errors the handlers and parsers send for it.
pub(crate) fn is_cut_short(message: &str) -> bool {
    message.contains(CUT_SHORT) || message == EMPTY_STREAM
}

/// The error a body ends with when it ends before its terminal event:
/// `cut_short` when any event came, the empty stream when none did.
pub(crate) fn ended_early(saw_event: bool, cut_short: &str) -> String {
    debug_assert!(
        is_cut_short(cut_short),
        "{cut_short:?} is a cut-short error"
    );
    match saw_event {
        true => cut_short.to_owned(),
        false => EMPTY_STREAM.to_owned(),
    }
}

/// The last line of a body that ended without a newline: the bytes left
/// after the last `\n`, as a line, when there are any and they decode
/// (#2249 review). A pump hands it to its handler before end of file, as a
/// whole-body read parses it.
pub(crate) fn last_line(carry: &[u8]) -> Option<Result<&str, std::str::Utf8Error>> {
    match carry.is_empty() {
        true => None,
        false => Some(std::str::from_utf8(carry).map(|line| line.trim_end_matches('\r'))),
    }
}

/// A reply that ended before its terminal event, with the usage its
/// provider reported before it did (#2249 review).
#[derive(Debug)]
pub(crate) struct UnfinishedReply {
    pub error: DomainError,
    pub usage: Option<UsageInfo>,
}

impl UnfinishedReply {
    /// A failure with nothing to account.
    pub(crate) fn failed(error: DomainError) -> Self {
        Self { error, usage: None }
    }

    /// The reply's error, its usage (priced for `model`) recorded on
    /// `trace` first so the tokens spent are still counted.
    pub(crate) fn account(self, trace: Option<&RequestTrace>, model: &str) -> DomainError {
        record_unfinished_usage(trace, self.usage, model);
        self.error
    }

    /// The reply's error alone (the tests' parse, which accounts nothing).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn into_error(self) -> DomainError {
        self.error
    }
}

impl From<DomainError> for UnfinishedReply {
    fn from(error: DomainError) -> Self {
        Self::failed(error)
    }
}

/// Record `usage` a cut-short reply reported on `trace`, priced for
/// `model`; nothing without both.
pub(crate) fn record_unfinished_usage(
    trace: Option<&RequestTrace>,
    usage: Option<UsageInfo>,
    model: &str,
) {
    if let (Some(trace), Some(usage)) = (trace, usage) {
        let mut priced = LlmResponse {
            content: None,
            tool_calls: Vec::new(),
            usage: Some(usage),
            stop_reason: None,
            thinking_blocks: Vec::new(),
        };
        crate::domain::inference::services::usage_accounting::attach_cost(&mut priced, model);
        let usage = priced.usage.expect("the usage just set is kept");
        trace.record_unfinished_usage(usage);
    }
}

#[cfg(test)]
#[path = "sse_end_tests.rs"]
mod tests;
