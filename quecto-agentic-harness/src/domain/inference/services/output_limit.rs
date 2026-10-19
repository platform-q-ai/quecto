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
use crate::domain::inference::services::provider_error::EMPTY_STREAM;

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
    /// The call's id, which its answer names.
    pub id: &'a str,
    /// The tool the call named.
    pub tool: &'a str,
}

/// The tool call of `response` the output limit cut off. The limit stops
/// a reply at its end, so only the reply's last call can be cut off: the
/// reply stopped at the output limit and that call's argument text is not
/// a whole JSON object, or there is none at all (the limit stopped it
/// before any, or the adapter reported a call its provider marks
/// incomplete with none). A call whose arguments are whole is a call,
/// limit or not; an earlier broken call, or one without the limit, is a
/// malformed call (#2123), not a cut-off one.
pub fn cut_off_tool_call(response: &LlmResponse) -> Option<CutOffCall<'_>> {
    let at_the_limit = response.stop_reason == Some(StopReason::MaxTokens);
    at_the_limit
        .then(|| response.tool_calls.last())
        .flatten()
        .filter(|call| match call.argument_shape() {
            ToolArguments::Invalid(_) | ToolArguments::Empty => true,
            ToolArguments::Object(_) => false,
        })
        .map(|call| CutOffCall {
            id: call.id.as_str(),
            tool: call.name.as_str(),
        })
}

/// A reply that hit the output limit with nothing visible: no text and no
/// tool call, only reasoning (#2124). It is no answer, never a final one.
/// Some providers also report a full context window as `max_tokens`; a reply
/// that used under half of its output budget did not hit the output limit.
/// A zero count means the provider did not report one: judged as unknown.
pub fn is_cut_off_without_answer(response: &LlmResponse, max_tokens: u32) -> bool {
    let used_the_budget = response.usage.as_ref().is_none_or(|usage| {
        usage.completion_tokens == 0
            || u64::from(usage.completion_tokens) * 2 >= u64::from(max_tokens)
    });
    response.stop_reason == Some(StopReason::MaxTokens)
        && response
            .content
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        && response.tool_calls.is_empty()
        && used_the_budget
}

/// The error of a completed reply with no output: the output limit when
/// it stopped there, else the synthetic empty stream (retried as
/// [`EMPTY_STREAM`]).
pub fn empty_stream_error_message(response: &LlmResponse) -> String {
    match response.stop_reason {
        Some(StopReason::MaxTokens) => {
            "stream completed without assistant output: stop_reason=max_tokens".to_string()
        }
        _ => EMPTY_STREAM.to_string(),
    }
}

#[cfg(test)]
#[path = "output_limit_tests.rs"]
mod tests;
