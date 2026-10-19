//! Recovery from transient inference failures (#1908): the policy for what
//! one agent run does after a reply it cannot take as it is, or a failed
//! request.
//!
//! - A request the provider rejected as malformed, with nothing shown, is
//!   corrected with feedback while its budget lasts (#931, #2155).
//! - A reply cut off at the output limit with nothing visible is asked
//!   again, concisely, with room to answer (#2124).
//! - A reply cut off inside a tool call is neither run nor recorded; the
//!   model is told how to split the call (#2593). Past its budget the call
//!   is answered as invalid (#2123) and the turn goes on.
//! - Anything else ends the turn with the failure; the guidance it carries
//!   is the domain's (`terminal_guidance`).
//!
//! Transport retries are not here: the provider decorators and the loop's
//! stream initiation own them. This policy owns the corrections, their
//! budgets and the feedback messages they add, and withdraws the feedback
//! whose correction did not succeed when the turn fails (#2132). It is pure:
//! the conversation it corrects is passed in, and the agent loop runs the
//! requests, the logs, the audit and the progress events.
use crate::domain::conversation::services::turn_origin::current_phase;
use crate::domain::conversation::value_objects::message::{LlmResponse, Message, Role};
use crate::domain::inference::services::output_limit::{
    cut_off_tool_call, empty_stream_error_message, is_cut_off_without_answer,
};
use crate::domain::inference::services::provider_error::{
    ProviderErrorClass, classify_provider_error, is_context_or_output_limit_error, model_refusal,
    provider_http_status,
};
use crate::domain::inference::services::terminal_guidance::Output;
use crate::domain::shared::error::DomainError;

/// Model-malformed requests corrected with feedback per run (#931).
pub const MAX_MALFORMED_REQUEST_RETRIES: u32 = 3;
/// Consecutive replies cut off with nothing visible asked again (#2124).
pub const MAX_CUT_OFF_RETRIES: u32 = 1;
/// Consecutive replies cut off inside a tool call corrected (#2593).
pub const MAX_TOOL_CALL_CUT_OFF_RETRIES: u32 = 2;

/// What the turn does next.
#[derive(Debug)]
pub enum Recovery {
    /// Take the reply as it is.
    Proceed,
    /// Add `feedback` for the model ([`InferenceRecovery::correct`])
    /// and ask again; `boost_output` lets the next request go above the
    /// effective output limit (#2124); `correction` says which, for the log.
    Correct {
        feedback: String,
        boost_output: bool,
        correction: Correction,
    },
    /// End the turn with this failure, once the feedback whose correction
    /// did not succeed is withdrawn ([`InferenceRecovery::withdraw`]).
    Fail(DomainError),
}

/// Which correction a [`Recovery::Correct`] is, and its attempt within
/// its budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Correction {
    /// A request the provider rejected as malformed (#931).
    MalformedRequest { attempt: u32, error: String },
    /// A reply cut off at the output limit with nothing visible (#2124).
    CutOffAnswer { attempt: u32 },
    /// A reply cut off inside its last tool call (#2593).
    CutOffCall { attempt: u32, tool: String },
}

/// What follows a failed provider request (#2155): a malformed request
/// whose reply showed nothing is corrected while the budget lasts;
/// everything else ends the turn with its class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderFailureTransition {
    RecoverMalformedRequest,
    Terminal(ProviderErrorClass),
}

/// One run's corrections: their budgets, and the feedback not yet settled
/// by an accepted reply.
#[derive(Debug, Default)]
pub struct InferenceRecovery {
    malformed_retries: u32,
    cut_off_retries: u32,
    tool_cut_off_retries: u32,
    /// Feedback added since the last accepted reply: what a failed turn
    /// withdraws (#2132).
    unsettled: Vec<uuid::Uuid>,
}

impl InferenceRecovery {
    /// A run's recovery, every budget whole.
    pub fn for_run() -> Self {
        Self::default()
    }

    /// What follows a completed reply, judged against the output limit its
    /// request was sent with.
    pub fn after_reply(&mut self, response: &LlmResponse, max_tokens: u32) -> Recovery {
        if is_cut_off_without_answer(response, max_tokens) {
            return self.after_cut_off_answer(response, max_tokens);
        }
        if let Some(cut) = cut_off_tool_call(response) {
            if self.tool_cut_off_retries < MAX_TOOL_CALL_CUT_OFF_RETRIES {
                self.tool_cut_off_retries += 1;
                return Recovery::Correct {
                    feedback: tool_call_cut_off_feedback(cut.tool),
                    boost_output: true,
                    correction: Correction::CutOffCall {
                        attempt: self.tool_cut_off_retries,
                        tool: cut.tool.to_string(),
                    },
                };
            }
        } else {
            self.tool_cut_off_retries = 0;
        }
        // An accepted reply settles the feedback that led to it.
        self.cut_off_retries = 0;
        self.unsettled.clear();
        Recovery::Proceed
    }

    /// What follows a failed request whose reply did or did not show
    /// `output`: a correction, or the end of the turn. Never `Proceed`.
    pub fn after_failure(&mut self, error: DomainError, output: Output) -> Recovery {
        let transition = classify_provider_failure(
            &error,
            output,
            self.malformed_retries,
            MAX_MALFORMED_REQUEST_RETRIES,
        );
        match transition {
            ProviderFailureTransition::RecoverMalformedRequest => {
                self.malformed_retries += 1;
                Recovery::Correct {
                    feedback: malformed_feedback(&error),
                    boost_output: false,
                    correction: Correction::MalformedRequest {
                        attempt: self.malformed_retries,
                        error: error.to_string(),
                    },
                }
            }
            ProviderFailureTransition::Terminal(_) => Recovery::Fail(error),
        }
    }

    /// Add `feedback` for the model as its own message (#2403), stamped
    /// with the open turn, and record it in the run's ledger (#1072). A
    /// message already sent is never edited, so the next request extends
    /// the last one. The stamp matters: the context pruner treats the last
    /// turn-less user message as the in-flight prompt, and unstamped
    /// feedback would usurp that role.
    pub fn correct(
        &mut self,
        messages: &mut Vec<Message>,
        ledger: &mut Vec<Message>,
        feedback: String,
        current_turn: u32,
    ) {
        let mut message = Message::user(feedback);
        message.turn = Some(current_turn);
        // The loop's own feedback continues the open turn (#2226).
        message.turn_origin = current_phase(messages);
        debug_assert_eq!(message.role, Role::User, "feedback is a user message");
        self.unsettled.push(message.id());
        ledger.push(message.clone());
        messages.push(message);
    }

    /// The turn failed: withdraw the feedback whose correction did not
    /// succeed from the conversation and the ledger (#2132). Feedback a
    /// pruning pass already dropped is simply gone.
    pub fn withdraw(&mut self, messages: &mut Vec<Message>, ledger: &mut Vec<Message>) {
        let unsettled = std::mem::take(&mut self.unsettled);
        if unsettled.is_empty() {
            return;
        }
        messages.retain(|m| !unsettled.contains(&m.id()));
        ledger.retain(|m| !unsettled.contains(&m.id()));
        assert!(
            messages
                .iter()
                .chain(ledger.iter())
                .all(|m| !unsettled.contains(&m.id())),
            "no withdrawn feedback stays in the conversation or the ledger"
        );
    }

    /// A reply cut off at the output limit with nothing visible (#2124):
    /// asked again, concisely, while the budget lasts; then the turn fails
    /// instead of ending empty.
    fn after_cut_off_answer(&mut self, response: &LlmResponse, max_tokens: u32) -> Recovery {
        if self.cut_off_retries < MAX_CUT_OFF_RETRIES {
            self.cut_off_retries += 1;
            return Recovery::Correct {
                feedback: output_limit_feedback(max_tokens),
                boost_output: true,
                correction: Correction::CutOffAnswer {
                    attempt: self.cut_off_retries,
                },
            };
        }
        Recovery::Fail(DomainError::Provider(empty_stream_error_message(response)))
    }
}

/// What follows a failed provider request. Only a malformed request (an
/// HTTP 400 or 422, or a 5xx wrapping an explicit `invalid_request_error`,
/// #935) whose reply showed nothing is repaired and sent again, while the
/// budget lasts; everything else ends the turn (#2155).
pub fn classify_provider_failure(
    error: &DomainError,
    output: Output,
    malformed_retries: u32,
    max_malformed_retries: u32,
) -> ProviderFailureTransition {
    let class = classify_provider_error(error);
    // A model the provider refused for the account (#2435) is no
    // malformed request: no repair makes the provider accept it.
    let is_malformed_request = matches!(error, DomainError::Provider(message)
        if class == ProviderErrorClass::Client
            && matches!(provider_http_status(error), Some(400 | 422 | 500..=599))
            && !is_context_or_output_limit_error(message)
            && model_refusal(error).is_none());
    if is_malformed_request
        && output == Output::NotShown
        && malformed_retries < max_malformed_retries
    {
        ProviderFailureTransition::RecoverMalformedRequest
    } else {
        ProviderFailureTransition::Terminal(class)
    }
}

/// Feedback for a request the provider rejected as malformed (#931 AC2).
fn malformed_feedback(err: &DomainError) -> String {
    format!(
        "Your previous request was rejected by the provider as malformed (not retryable): {err}\n\nPlease correct the request — for example fix any malformed tool call arguments or invalid fields — and try again.",
    )
}

/// Feedback for a reply that hit the output limit before anything visible
/// (#2124): no text and no tool call, typically all reasoning.
fn output_limit_feedback(max_tokens: u32) -> String {
    format!(
        "Your previous reply hit the output limit ({max_tokens} tokens) before any answer or \
         complete tool call. Reply concisely: either one complete tool call or a short final \
         answer, and keep any reasoning brief."
    )
}

/// Feedback for a reply the output limit cut off partway through a call's
/// arguments (#2593): no call of the reply ran, and how to fit the work
/// in. No number: on some paths (Codex behind a ChatGPT sign-in) the
/// request's limit is never sent, so the provider's limit is not known.
fn tool_call_cut_off_feedback(tool: &str) -> String {
    format!(
        "Your previous reply hit the output limit partway through the arguments of a `{tool}` \
         call, so the reply was cut off and none of its tool calls were run. Keep each call \
         small enough to finish within the limit: split large content across several calls \
         (for a large file, write the first part, then add the rest with `edit`)."
    )
}

#[cfg(test)]
#[path = "inference_recovery_tests.rs"]
mod tests;
