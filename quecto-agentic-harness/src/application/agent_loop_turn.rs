use super::is_context_or_output_limit_error;
use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider_error::{ProviderErrorClass, classify_provider_error};

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

pub(super) fn classify_provider_failure(
    error: &DomainError,
    malformed_retries: u32,
    max_malformed_retries: u32,
) -> ProviderFailureTransition {
    let class = classify_provider_error(error);
    let is_malformed_request = matches!(error, DomainError::Provider(message)
        if class == ProviderErrorClass::Client && !is_context_or_output_limit_error(message));

    if is_malformed_request && malformed_retries < max_malformed_retries {
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
