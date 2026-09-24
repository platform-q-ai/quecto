use crate::application::agent_usage::UsageTotals;
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, StopReason};

pub(super) struct StreamProviderError {
    pub(super) error: DomainError,
    pub(super) emitted_event: bool,
}

pub(super) struct TurnEnd {
    pub(super) iterations: u32,
    pub(super) usage: UsageTotals,
    pub(super) pre_response_context_tokens: usize,
    pub(super) current_turn: u32,
}

pub(super) fn is_empty_streamed_response(response: &LlmResponse) -> bool {
    response.content.as_deref().unwrap_or_default().is_empty()
        && response.tool_calls.is_empty()
        && response.thinking_blocks.is_empty()
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
        _ => "stream completed without assistant output: synthetic=empty_stream".to_string(),
    }
}

#[cfg(test)]
#[path = "agent_loop_stream_tests.rs"]
mod agent_loop_stream_tests;
