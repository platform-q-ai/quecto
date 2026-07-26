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

pub(super) fn empty_stream_error_message(response: &LlmResponse) -> String {
    match response.stop_reason {
        Some(StopReason::MaxTokens) => {
            "stream completed without assistant output: stop_reason=max_tokens".to_string()
        }
        _ => "HTTP 503: stream completed without assistant output".to_string(),
    }
}

#[cfg(test)]
#[path = "agent_loop_stream_tests.rs"]
mod agent_loop_stream_tests;
