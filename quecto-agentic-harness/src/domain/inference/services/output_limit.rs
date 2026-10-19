//! The output limit (#2593): which provider signals say a reply was cut
//! off at it, and when such a reply cut off a tool call partway through
//! its arguments.
//!
//! Every adapter reports a cut-off reply as a finished reply that stopped
//! at [`StopReason::MaxTokens`]: Anthropic's `max_tokens`, OpenAI's
//! `length` ([`StopReason::from_openai_finish_reason`]) and, for the
//! Responses API, an incomplete response whose reason is in
//! [`RESPONSES_OUTPUT_LIMIT_REASONS`]. A cut-off reply is never a provider
//! error: the agent turn decides what follows it.
use crate::domain::conversation::value_objects::message::{LlmResponse, StopReason, ToolArguments};

/// The Responses API `incomplete_details.reason` values that mean the
/// reply was cut off at its output limit. An allowlist: any other reason
/// (a content filter, one added later) stays the failure it is.
pub const RESPONSES_OUTPUT_LIMIT_REASONS: &[&str] = &["max_output_tokens"];

/// Whether a Responses API incomplete `reason` is the output limit.
pub fn is_responses_output_limit(reason: &str) -> bool {
    RESPONSES_OUTPUT_LIMIT_REASONS.contains(&reason)
}

/// A tool call the output limit cut off partway through its arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CutOffCall<'a> {
    /// The tool the call named.
    pub tool: &'a str,
}

/// The first tool call of `response` the output limit cut off: the reply
/// stopped at the output limit and the call's argument text is not a
/// whole JSON object, or the call is the reply's last and got no argument
/// text at all (the limit stopped it before any, or the adapter reported
/// a call its provider marks incomplete with none). A call whose arguments
/// are whole is a call, limit or not; invalid arguments without the limit
/// are a malformed call (#2123), not a cut-off one.
pub fn cut_off_tool_call(response: &LlmResponse) -> Option<CutOffCall<'_>> {
    if response.stop_reason != Some(StopReason::MaxTokens) {
        return None;
    }
    let last = response.tool_calls.len().checked_sub(1);
    response
        .tool_calls
        .iter()
        .enumerate()
        .find(|(index, call)| match call.argument_shape() {
            ToolArguments::Invalid(_) => true,
            ToolArguments::Empty => Some(*index) == last,
            ToolArguments::Object(_) => false,
        })
        .map(|(_, call)| CutOffCall {
            tool: call.name.as_str(),
        })
}

#[cfg(test)]
#[path = "output_limit_tests.rs"]
mod tests;
