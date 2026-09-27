use super::is_context_or_output_limit_error;
use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider_error::{
    ProviderErrorClass, classify_provider_error, provider_http_status,
};

/// Internal vocabulary for the agent turn lifecycle.
///
/// These states intentionally describe the orchestration inside one public
/// `AgentLoop::process` call without becoming protocol-visible API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnState {
    PrepareProviderRequest,
    AwaitProviderResponse,
    RecoverMalformedResponse,
    ExecuteToolCalls,
    FinalizeAssistantResponse,
    FailProviderRequest,
    StopAtToolIterationLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderResponseTransition {
    FinalAssistantResponse,
    ToolCallContinuation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProviderFailureTransition {
    RecoverMalformedRequest,
    Terminal(ProviderErrorClass),
}

pub(super) fn classify_provider_response(response: &LlmResponse) -> ProviderResponseTransition {
    if response.tool_calls.is_empty() {
        ProviderResponseTransition::FinalAssistantResponse
    } else {
        ProviderResponseTransition::ToolCallContinuation
    }
}

/// Whether a failed request had already shown output to the user (#2155).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Output {
    /// Nothing reached the user: the request may be sent again.
    NotShown,
    /// Text, thinking or a tool call already streamed: sending the request
    /// again would show a second reply after the first one's fragment.
    Shown,
}

impl Output {
    /// The output state of a stream that did (`true`) or did not emit events.
    pub(super) fn from_emitted(emitted_event: bool) -> Self {
        match emitted_event {
            true => Self::Shown,
            false => Self::NotShown,
        }
    }
}

/// What follows a failed provider request. Only a malformed request (an
/// HTTP 400 or 422, or a 5xx wrapping an explicit `invalid_request_error`,
/// #935) whose reply showed nothing is repaired and sent again, while the
/// budget lasts; everything else ends the turn (#2155).
pub(super) fn classify_provider_failure(
    error: &DomainError,
    output: Output,
    malformed_retries: u32,
    max_malformed_retries: u32,
) -> ProviderFailureTransition {
    let class = classify_provider_error(error);
    let is_malformed_request = matches!(error, DomainError::Provider(message)
        if class == ProviderErrorClass::Client
            && matches!(provider_http_status(error), Some(400 | 422 | 500..=599))
            && !is_context_or_output_limit_error(message));

    if is_malformed_request
        && output == Output::NotShown
        && malformed_retries < max_malformed_retries
    {
        ProviderFailureTransition::RecoverMalformedRequest
    } else {
        ProviderFailureTransition::Terminal(class)
    }
}

pub(super) fn next_state_after_provider_response(response: &LlmResponse) -> TurnState {
    match classify_provider_response(response) {
        ProviderResponseTransition::FinalAssistantResponse => TurnState::FinalizeAssistantResponse,
        ProviderResponseTransition::ToolCallContinuation => TurnState::ExecuteToolCalls,
    }
}

pub(super) fn state_for_provider_failure_transition(
    transition: &ProviderFailureTransition,
) -> TurnState {
    match transition {
        ProviderFailureTransition::RecoverMalformedRequest => TurnState::RecoverMalformedResponse,
        ProviderFailureTransition::Terminal(_) => TurnState::FailProviderRequest,
    }
}
