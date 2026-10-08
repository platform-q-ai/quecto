use crate::application::agent_usage::UsageTotals;
use crate::domain::conversation::services::reply_requirement::ReplyRequirement;
use crate::domain::conversation::value_objects::message::{LlmResponse, StopReason};
use crate::domain::error::DomainError;

/// A failed provider request, and whether its reply had already emitted
/// events (shown output) before it failed.
pub(super) struct StreamProviderError {
    pub(super) error: DomainError,
    pub(super) emitted_event: bool,
}

impl StreamProviderError {
    /// A failure before any output was shown.
    pub(super) fn before_output(error: DomainError) -> Self {
        Self {
            error,
            emitted_event: false,
        }
    }
}

pub(super) struct TurnEnd {
    pub(super) iterations: u32,
    pub(super) usage: UsageTotals,
    pub(super) pre_response_context_tokens: usize,
    pub(super) current_turn: u32,
}

/// A reply with nothing in it: no text (whitespace alone is none, #2434),
/// no tool call and no visible thinking.
pub(super) fn is_empty_streamed_response(response: &LlmResponse) -> bool {
    response
        .content
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
        && response.tool_calls.is_empty()
        && !crate::domain::conversation::services::visible_thinking::has_visible_thinking(
            &response.thinking_blocks,
        )
}

/// A reply with nothing in it that ends the turn (#2434): it answers a
/// conversation that lets it be empty (tool results, never a prompt, a
/// steer or a follow-up) and it says it finished (`end_turn`). A reply
/// stopped at the output limit was cut off (#2124), and one that names no
/// stop reason is not known to have finished: both stay empty streams.
pub(super) fn ends_turn_empty(response: &LlmResponse, requirement: ReplyRequirement) -> bool {
    requirement == ReplyRequirement::MayBeEmpty
        && is_empty_streamed_response(response)
        // `EndTurn` also stands for Anthropic's `pause_turn` and
        // `stop_sequence` (`StopReason::parse`). Neither can stop a reply
        // today: the harness offers no server tools (which pause a turn)
        // and sets no stop sequences.
        && response.stop_reason == Some(StopReason::EndTurn)
}

/// A reply that hit the output limit with nothing visible: no text and no
/// tool call, only reasoning (#2124). It is no answer, never a final one.
/// Some providers also report a full context window as `max_tokens`; a reply
/// that used under half of its output budget did not hit the output limit.
/// A zero count means the provider did not report one: judged as unknown.
pub(super) fn is_cut_off_without_answer(response: &LlmResponse, max_tokens: u32) -> bool {
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

pub(super) fn empty_stream_error_message(response: &LlmResponse) -> String {
    match response.stop_reason {
        Some(StopReason::MaxTokens) => {
            "stream completed without assistant output: stop_reason=max_tokens".to_string()
        }
        _ => crate::domain::inference::services::provider_error::EMPTY_STREAM.to_string(),
    }
}

#[cfg(test)]
#[path = "agent_loop_stream_tests.rs"]
mod agent_loop_stream_tests;
