use super::agent_loop_turn::*;
use crate::domain::conversation::services::reply_requirement::ReplyRequirement;
use crate::domain::conversation::value_objects::message::{LlmResponse, ToolCall};
use crate::domain::error::DomainError;
use crate::domain::inference::services::provider_error::ProviderErrorClass;

fn next_state_after_provider_failure(
    error: &DomainError,
    malformed_retries: u32,
    max_malformed_retries: u32,
) -> TurnState {
    let transition = classify_provider_failure(
        error,
        Output::NotShown,
        malformed_retries,
        max_malformed_retries,
    );
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
        next_state_after_provider_response(&text_response(), ReplyRequirement::Output),
        TurnState::FinalizeAssistantResponse
    );
}

#[test]
fn tool_call_response_transitions_to_tool_continuation() {
    assert_eq!(
        next_state_after_provider_response(&tool_response(), ReplyRequirement::MayBeEmpty),
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
        classify_provider_failure(&err, Output::NotShown, 0, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Auth)
    );
    assert_eq!(
        next_state_after_provider_failure(&err, 0, 3),
        TurnState::FailProviderRequest
    );
}

#[test]
fn a_model_refused_for_the_account_is_terminal_not_repaired_as_malformed() {
    // #2435: no repair of the request makes the provider accept a model
    // the account cannot use; the turn ends at once with the refusal.
    let err = DomainError::Provider(
        r#"HTTP 400 from Codex: {"detail":"The 'mini' model is not supported when using Codex with a ChatGPT account."}"#
            .to_string(),
    );
    assert_eq!(
        classify_provider_failure(&err, Output::NotShown, 0, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Client)
    );
}

#[test]
fn admission_refusal_is_terminal_not_repaired_as_malformed() {
    // A refused admission is not a malformed request: the loop must not
    // spend its malformed-recovery budget re-sending it (#2024 S3).
    let err = DomainError::Provider(
        "admission: admission refused: capability revoked by an authority reset; a child cannot re-register on its own — its parent must respawn it".to_string(),
    );
    assert_eq!(
        classify_provider_failure(&err, Output::NotShown, 0, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Admission)
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
        classify_provider_failure(&err, Output::NotShown, 0, 3),
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
        next_state_after_provider_response(&response, ReplyRequirement::MayBeEmpty),
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
        classify_provider_failure(&err, Output::NotShown, 0, 0),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Client)
    );
}

#[test]
fn context_limit_client_failure_is_terminal_not_malformed_feedback() {
    let err =
        DomainError::Provider("provider error (400): maximum context length exceeded".to_string());
    assert_eq!(
        classify_provider_failure(&err, Output::NotShown, 0, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Client)
    );
}

#[test]
fn unknown_provider_failure_preserves_unknown_terminal_class() {
    let err = DomainError::Provider("provider returned a surprising failure".to_string());
    assert_eq!(
        classify_provider_failure(&err, Output::NotShown, 0, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Unknown)
    );
}

#[test]
fn malformed_recovery_budget_exhaustion_transitions_to_terminal_failure() {
    let err = DomainError::Provider(
        "provider error (400): invalid_request_error: tool_use malformed".to_string(),
    );
    assert_eq!(
        classify_provider_failure(&err, Output::NotShown, 3, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Client)
    );
}

/// #2155: a malformed request whose reply already showed output is never
/// repaired and sent again: the turn ends with it.
#[test]
fn malformed_failure_after_output_is_terminal() {
    let err = DomainError::Provider(
        "provider error (400): invalid_request_error: tool_use malformed".to_string(),
    );
    assert_eq!(
        classify_provider_failure(&err, Output::Shown, 0, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Client)
    );
    assert_eq!(
        classify_provider_failure(&err, Output::from_emitted(true), 0, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Client)
    );
    assert_eq!(
        classify_provider_failure(&err, Output::from_emitted(false), 0, 3),
        ProviderFailureTransition::RecoverMalformedRequest
    );
}

/// #2155 review: only a 400 or 422 (or a 5xx declaring an explicit
/// `invalid_request_error`, #935) is a malformed request; a missing model or
/// route (404) and the other client statuses end the turn.
#[test]
fn only_malformed_statuses_are_recovered() {
    for (message, recovered) in [
        (
            "HTTP 400 OpenAI stream error: {\"error\":{\"type\":\"invalid_request_error\"}}",
            true,
        ),
        ("HTTP 422 unprocessable entity", true),
        (
            "HTTP 500 from Fireworks: {\"error\":{\"code\":\"invalid_request_error\"}}",
            true,
        ),
        (
            "HTTP 404 OpenAI stream error: {\"error\":{\"type\":\"not_found_error\"}}",
            false,
        ),
        ("HTTP 405 method not allowed", false),
        ("HTTP 409 conflict", false),
        ("HTTP 410 gone", false),
        ("HTTP 406 not acceptable", false),
    ] {
        let err = DomainError::Provider(message.to_string());
        assert_eq!(
            classify_provider_failure(&err, Output::NotShown, 0, 3)
                == ProviderFailureTransition::RecoverMalformedRequest,
            recovered,
            "{message}"
        );
    }
}
