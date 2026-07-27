use super::agent_loop_turn::*;
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, ToolCall};
use crate::domain::provider_error::ProviderErrorClass;

fn next_state_after_provider_failure(
    error: &DomainError,
    malformed_retries: u32,
    max_malformed_retries: u32,
) -> TurnState {
    let transition = classify_provider_failure(error, malformed_retries, max_malformed_retries);
    state_for_provider_failure_transition(&transition)
}

fn text_response() -> LlmResponse {
    LlmResponse {
        content: Some("done".to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

fn tool_response() -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "read".to_string(),
            arguments: "{}".to_string(),
        }],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

#[test]
fn final_assistant_response_transitions_to_finalization() {
    assert_eq!(
        next_state_after_provider_response(&text_response()),
        TurnState::FinalizeAssistantResponse
    );
}

#[test]
fn tool_call_response_transitions_to_tool_continuation() {
    assert_eq!(
        next_state_after_provider_response(&tool_response()),
        TurnState::ExecuteToolCalls
    );
}

#[test]
fn malformed_client_failure_transitions_to_recovery_while_budget_remains() {
    let err = DomainError::Provider(
        "provider error (400): invalid_request_error: tool_use malformed".to_string(),
    );
    assert_eq!(
        next_state_after_provider_failure(&err, 0, 3),
        TurnState::RecoverMalformedResponse
    );
}

#[test]
fn provider_failure_transitions_to_terminal_when_not_addressable() {
    let err = DomainError::Provider("provider error (401): invalid credentials".to_string());
    assert_eq!(
        classify_provider_failure(&err, 0, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Auth)
    );
    assert_eq!(
        next_state_after_provider_failure(&err, 0, 3),
        TurnState::FailProviderRequest
    );
}

#[test]
fn cancelled_provider_failure_is_terminal_not_recovered() {
    let err = DomainError::Provider("request cancelled by caller".to_string());
    assert_eq!(
        classify_provider_failure(&err, 0, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Cancelled)
    );
    assert_eq!(
        next_state_after_provider_failure(&err, 0, 3),
        TurnState::FailProviderRequest
    );
}

#[test]
fn mixed_content_and_tool_calls_prefers_tool_continuation() {
    let mut response = tool_response();
    response.content = Some("I need a tool".to_string());

    assert_eq!(
        next_state_after_provider_response(&response),
        TurnState::ExecuteToolCalls
    );
}

#[test]
fn malformed_recovery_budget_boundary_allows_last_retry() {
    let err = DomainError::Provider(
        "provider error (400): invalid_request_error: tool_use malformed".to_string(),
    );
    assert_eq!(
        next_state_after_provider_failure(&err, 2, 3),
        TurnState::RecoverMalformedResponse
    );
}

#[test]
fn zero_malformed_recovery_budget_is_terminal() {
    let err = DomainError::Provider(
        "provider error (400): invalid_request_error: tool_use malformed".to_string(),
    );
    assert_eq!(
        classify_provider_failure(&err, 0, 0),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Client)
    );
}

#[test]
fn context_limit_client_failure_is_terminal_not_malformed_feedback() {
    let err =
        DomainError::Provider("provider error (400): maximum context length exceeded".to_string());
    assert_eq!(
        classify_provider_failure(&err, 0, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Client)
    );
}

#[test]
fn unknown_provider_failure_preserves_unknown_terminal_class() {
    let err = DomainError::Provider("provider returned a surprising failure".to_string());
    assert_eq!(
        classify_provider_failure(&err, 0, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Unknown)
    );
}

#[test]
fn malformed_recovery_budget_exhaustion_transitions_to_terminal_failure() {
    let err = DomainError::Provider(
        "provider error (400): invalid_request_error: tool_use malformed".to_string(),
    );
    assert_eq!(
        classify_provider_failure(&err, 3, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Client)
    );
}
