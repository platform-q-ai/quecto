//! The agent loop's provider-failure records and correction feedback (#931):
//! the audit event a terminal failure persists, and the feedback a
//! malformed or cut-off reply is corrected with. The guidance a terminal
//! failure gets is the domain's (`terminal_guidance`).

use crate::domain::audit::AuditEvent;
use crate::domain::conversation::value_objects::message::{Message, Role};
use crate::domain::inference::services::provider_error::{
    classify_provider_error, provider_http_status,
};
use crate::domain::shared::error::DomainError;

/// Build the audit event that persists a terminal provider failure (#937).
///
/// Returns the rich [`AuditEvent::ProviderError`] carrying the *full*,
/// untruncated error body (with provider name, classified class, and recovered
/// HTTP status). The body is redacted by [`AuditEvent::provider_error`] before
/// it is persisted, so a failed turn is retrievable from `~/.quecto/audit`
/// instead of evaporating with the truncated TUI line.
///
/// Only this single event is emitted. An earlier version also emitted a generic
/// [`AuditEvent::Error`] carrying the same `err.to_string()` "for back-compat",
/// but that copy was *not* redacted, so any secret echoed in the body landed on
/// disk in cleartext on the Error line right next to the scrubbed ProviderError
/// (PR #939 security review). No consumer required the duplicate, so it is
/// dropped: the redacted ProviderError is the sole, complete record. Emitted
/// once per terminal failure (the caller invokes this only after
/// retries/malformed re-prompts are exhausted), never per retry.
pub(super) fn provider_failure_audit_event(provider: &str, err: &DomainError) -> AuditEvent {
    AuditEvent::provider_error(
        provider,
        &classify_provider_error(err),
        provider_http_status(err),
        &err.to_string(),
    )
}

/// Append addressable "your request was malformed, please fix it" feedback so a
/// model-malformed request becomes a correctable next turn rather than a fatal
/// error (#931 AC2).
///
/// The feedback goes in as its own message (#2403): a message already sent
/// is never edited, so the next request still extends the last one. It is
/// stamped with `current_turn`: the context pruner treats the *last
/// turn-less user message* as the in-flight prompt (never dropped, marks the
/// current-prompt boundary), and an unstamped mid-prompt feedback message
/// would usurp that role — stripping tail-pin protection from the real
/// prompt and the most recent turns.
pub(super) fn append_malformed_feedback(
    messages: &mut Vec<Message>,
    err: &DomainError,
    current_turn: u32,
) {
    let feedback = format!(
        "Your previous request was rejected by the provider as malformed (not retryable): {err}\n\nPlease correct the request — for example fix any malformed tool call arguments or invalid fields — and try again.",
    );
    append_feedback(messages, feedback, current_turn)
}

/// Records the feedback the run just added in its ledger (#1072, #2124):
/// the run added that message, so it is appended once.
pub(super) fn record_feedback(messages: &[Message], ledger: &mut Vec<Message>) {
    let added = messages.last().expect("the feedback was just appended");
    debug_assert_eq!(added.role, Role::User, "feedback is a user message");
    ledger.push(added.clone());
}

/// Feedback for a reply that hit the output limit before anything visible
/// (#2124): no text and no tool call, typically all reasoning.
pub(super) fn output_limit_feedback(max_tokens: u32) -> String {
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
pub(super) fn tool_call_cut_off_feedback(tool: &str) -> String {
    format!(
        "Your previous reply hit the output limit partway through the arguments of a `{tool}` \
         call, so the reply was cut off and none of its tool calls were run. Keep each call \
         small enough to finish within the limit: split large content across several calls \
         (for a large file, write the first part, then add the rest with `edit`)."
    )
}

/// Adds `feedback` for the model as its own message (#2403): a message
/// already sent is never edited, so the next request extends the last one.
pub(super) fn append_feedback(messages: &mut Vec<Message>, feedback: String, current_turn: u32) {
    let mut msg = Message::user(feedback);
    msg.turn = Some(current_turn);
    // The loop's own feedback continues the open turn (#2226).
    msg.turn_origin = crate::domain::conversation::services::turn_origin::current_phase(messages);
    messages.push(msg);
}

#[cfg(test)]
#[path = "agent_loop_errors_tests.rs"]
mod tests;
