//! The recovery use case (#1908): the corrections it chooses, their
//! budgets, the feedback it adds and withdraws (#2132), and the
//! failure classification it corrects by (#931, #2155, migrated from the
//! agent loop's turn tests).
use super::*;
use crate::domain::conversation::services::turn_origin::{TurnOrigin, instruction, progress_nudge};
use crate::domain::conversation::value_objects::message::{StopReason, ThinkingBlock, ToolCall};

fn reply(stop: Option<StopReason>, calls: &[(&str, &str)], thinking: bool) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: calls
            .iter()
            .map(|(name, arguments)| ToolCall {
                id: format!("call_{name}"),
                name: (*name).into(),
                arguments: (*arguments).into(),
            })
            .collect(),
        usage: None,
        stop_reason: stop,
        thinking_blocks: if thinking {
            vec![ThinkingBlock::Normal {
                thinking: "long".into(),
                signature: String::new(),
            }]
        } else {
            vec![]
        },
    }
}

fn text(content: &str) -> LlmResponse {
    LlmResponse {
        content: Some(content.into()),
        ..reply(None, &[], false)
    }
}

fn cut_write() -> LlmResponse {
    reply(
        Some(StopReason::MaxTokens),
        &[("write", r#"{"path":"a","content":"li"#)],
        false,
    )
}

fn malformed() -> DomainError {
    DomainError::Provider(
        "provider error (400): invalid_request_error: tool_use malformed".to_string(),
    )
}

fn feedback_of(recovery: Recovery) -> (String, bool) {
    match recovery {
        Recovery::Correct {
            feedback,
            boost_output,
        } => (feedback, boost_output),
        other => panic!("expected a correction, got {other:?}"),
    }
}

#[test]
fn a_whole_reply_proceeds() {
    let mut recovery = RecoverInferenceFailure::for_run();
    assert!(matches!(
        recovery.after_reply(&text("done"), 8192),
        Recovery::Proceed
    ));
}

#[test]
fn a_reasoning_only_cut_off_is_corrected_once_with_room_then_fails() {
    let mut recovery = RecoverInferenceFailure::for_run();
    let cut = reply(Some(StopReason::MaxTokens), &[], true);
    let (feedback, boost) = feedback_of(recovery.after_reply(&cut, 8192));
    assert!(
        feedback.contains("output limit (8192 tokens)"),
        "{feedback}"
    );
    assert!(boost, "the retry may use the model's cap");
    match recovery.after_reply(&cut, 8192) {
        Recovery::Fail(DomainError::Provider(message)) => {
            assert_eq!(
                message,
                "stream completed without assistant output: stop_reason=max_tokens"
            );
        }
        other => panic!("expected the turn to fail, got {other:?}"),
    }
}

#[test]
fn a_cut_off_call_is_corrected_twice_then_proceeds_as_invalid() {
    let mut recovery = RecoverInferenceFailure::for_run();
    for _ in 0..MAX_TOOL_CALL_CUT_OFF_RETRIES {
        let (feedback, boost) = feedback_of(recovery.after_reply(&cut_write(), 8192));
        assert!(feedback.contains("`write`") && boost, "{feedback}");
    }
    assert!(matches!(
        recovery.after_reply(&cut_write(), 8192),
        Recovery::Proceed
    ));
    assert!(
        matches!(recovery.after_reply(&cut_write(), 8192), Recovery::Proceed),
        "a cut-off answered as invalid keeps the budget spent"
    );
    assert!(matches!(
        recovery.after_reply(&text("ok"), 8192),
        Recovery::Proceed
    ));
    assert!(
        matches!(
            recovery.after_reply(&cut_write(), 8192),
            Recovery::Correct { .. }
        ),
        "an accepted reply restores the budget"
    );
}

#[test]
fn a_malformed_request_is_corrected_without_a_boost_until_the_budget_runs_out() {
    let mut recovery = RecoverInferenceFailure::for_run();
    for _ in 0..MAX_MALFORMED_REQUEST_RETRIES {
        let (feedback, boost) = feedback_of(recovery.after_failure(malformed(), Output::NotShown));
        assert!(feedback.contains("rejected by the provider as malformed"));
        assert!(!boost);
    }
    assert!(matches!(
        recovery.after_failure(malformed(), Output::NotShown),
        Recovery::Fail(_)
    ));
}

#[test]
fn a_failure_after_output_or_of_another_class_fails_at_once() {
    let mut recovery = RecoverInferenceFailure::for_run();
    assert!(matches!(
        recovery.after_failure(malformed(), Output::Shown),
        Recovery::Fail(_)
    ));
    let auth = DomainError::Provider("provider error (401): invalid credentials".into());
    assert!(matches!(
        recovery.after_failure(auth, Output::NotShown),
        Recovery::Fail(_)
    ));
}

/// #2226, #2403: feedback is its own message, stamped with the open turn
/// and the origin of the turn already open; a sent message is never edited.
#[test]
fn feedback_is_its_own_message_with_the_open_turns_origin() {
    for (opener, origin) in [
        (progress_nudge("continue".into()), TurnOrigin::ProgressNudge),
        (instruction("task".into()), TurnOrigin::Instruction),
    ] {
        let mut recovery = RecoverInferenceFailure::for_run();
        let mut messages = vec![opener.clone()];
        let mut ledger = Vec::new();
        recovery.correct(&mut messages, &mut ledger, "try again".into(), 3);
        assert_eq!(messages.len(), 2, "added as its own message");
        assert_eq!(messages[0].content, opener.content, "never edited");
        let feedback = messages.last().unwrap();
        assert_eq!(feedback.role, Role::User);
        assert_eq!(feedback.turn_origin, origin);
        assert_eq!(feedback.turn, Some(3));
        assert_eq!(ledger.len(), 1, "the run added it, once");
        assert_eq!(ledger[0].id(), feedback.id());
    }
    let mut recovery = RecoverInferenceFailure::for_run();
    let mut unstamped = vec![Message::assistant("", vec![])];
    recovery.correct(&mut unstamped, &mut Vec::new(), "try again".into(), 1);
    assert_eq!(unstamped[1].turn_origin, TurnOrigin::Unknown);
}

/// #2132: a failed turn withdraws the feedback its accepted replies did not
/// settle, from the conversation and the ledger; settled feedback stays.
#[test]
fn withdraw_removes_only_unsettled_feedback() {
    let mut recovery = RecoverInferenceFailure::for_run();
    let mut messages = vec![Message::user("task")];
    let mut ledger = Vec::new();
    recovery.correct(&mut messages, &mut ledger, "settled".into(), 1);
    assert!(matches!(
        recovery.after_reply(&text("ok"), 8192),
        Recovery::Proceed
    ));
    recovery.correct(&mut messages, &mut ledger, "unsettled one".into(), 2);
    recovery.correct(&mut messages, &mut ledger, "unsettled two".into(), 3);
    recovery.withdraw(&mut messages, &mut ledger);
    let left: Vec<&str> = messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(left, ["task", "settled"]);
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].content, "settled");
    // Withdrawn once: nothing is left to withdraw.
    recovery.withdraw(&mut messages, &mut ledger);
    assert_eq!(messages.len(), 2);
}

/// #2132: feedback a pruning pass already dropped is simply gone.
#[test]
fn withdraw_tolerates_feedback_already_pruned() {
    let mut recovery = RecoverInferenceFailure::for_run();
    let mut messages = vec![Message::user("task")];
    let mut ledger = Vec::new();
    recovery.correct(&mut messages, &mut ledger, "pruned".into(), 1);
    messages.pop();
    recovery.withdraw(&mut messages, &mut ledger);
    assert_eq!(messages.len(), 1);
    assert!(ledger.is_empty());
}

#[test]
fn malformed_client_failure_transitions_to_recovery_while_budget_remains() {
    let err = DomainError::Provider(
        "provider error (400): invalid_request_error: tool_use malformed".to_string(),
    );
    assert_eq!(
        classify_provider_failure(&err, Output::NotShown, 0, 3),
        ProviderFailureTransition::RecoverMalformedRequest
    );
}

#[test]
fn provider_failure_transitions_to_terminal_when_not_addressable() {
    let err = DomainError::Provider("provider error (401): invalid credentials".to_string());
    assert_eq!(
        classify_provider_failure(&err, Output::NotShown, 0, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Auth)
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
}

#[test]
fn cancelled_provider_failure_is_terminal_not_recovered() {
    let err = DomainError::Provider("request cancelled by caller".to_string());
    assert_eq!(
        classify_provider_failure(&err, Output::NotShown, 0, 3),
        ProviderFailureTransition::Terminal(ProviderErrorClass::Cancelled)
    );
}

#[test]
fn malformed_recovery_budget_boundary_allows_last_retry() {
    let err = DomainError::Provider(
        "provider error (400): invalid_request_error: tool_use malformed".to_string(),
    );
    assert_eq!(
        classify_provider_failure(&err, Output::NotShown, 2, 3),
        ProviderFailureTransition::RecoverMalformedRequest
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
