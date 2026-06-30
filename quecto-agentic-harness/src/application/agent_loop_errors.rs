//! Terminal provider-error enhancement for the agent loop (#931).
//!
//! After retries are exhausted (or for a non-retryable error), turn a raw
//! provider string into a classified, actionable message: name the error class
//! and the remediation so the agent/parent can react sensibly.

use crate::domain::audit::AuditEvent;
use crate::domain::error::DomainError;
use crate::domain::message::{Message, Role};
use crate::domain::provider_error::{
    ProviderErrorClass, classify_provider_error, provider_http_status,
};

/// Build the audit events that persist a terminal provider failure (#937).
///
/// Returns, in emit order, the rich [`AuditEvent::ProviderError`] carrying the
/// *full*, untruncated error body (with provider name, classified class, and
/// recovered HTTP status) plus the generic [`AuditEvent::Error`] for back-compat
/// consumers. The ProviderError body is redacted by
/// [`AuditEvent::provider_error`] before it is persisted, so a failed turn is
/// retrievable from `~/.quecto/audit` instead of evaporating with the truncated
/// TUI line. Emitted once per terminal failure (the caller invokes this only
/// after retries/malformed re-prompts are exhausted), never per retry.
pub(super) fn provider_failure_audit_events(provider: &str, err: &DomainError) -> [AuditEvent; 2] {
    [
        AuditEvent::provider_error(
            provider,
            &classify_provider_error(err),
            provider_http_status(err),
            &err.to_string(),
        ),
        AuditEvent::Error {
            source: "provider".into(),
            tool: None,
            message: err.to_string(),
        },
    ]
}

/// Append addressable "your request was malformed, please fix it" feedback so a
/// model-malformed request becomes a correctable next turn rather than a fatal
/// error (#931 AC2).
///
/// The rejection happens before any assistant message is added this turn, so the
/// trailing message is often already a user / tool-result. Appending a second
/// consecutive `user` turn is itself rejected as a 400 by some providers
/// (Anthropic), which would re-enter this branch and burn the retry budget
/// without the model ever self-correcting. Merge into the trailing user message
/// when there is one; otherwise push a fresh one.
pub(super) fn append_malformed_feedback(messages: &mut Vec<Message>, err: &DomainError) {
    let feedback = format!(
        "Your previous request was rejected by the provider as malformed (not retryable): {err}\n\nPlease correct the request — for example fix any malformed tool call arguments or invalid fields — and try again.",
    );
    match messages.last_mut() {
        Some(last) if last.role == Role::User => {
            last.content.push_str("\n\n");
            last.content.push_str(&feedback);
        }
        _ => messages.push(Message::user(feedback)),
    }
}

pub(super) fn enhance_provider_error(err: DomainError) -> DomainError {
    let DomainError::Provider(message) = err else {
        return err;
    };

    if is_context_or_output_limit_error(&message)
        && !message
            .to_ascii_lowercase()
            .contains("context/output limit")
    {
        return DomainError::Provider(format!(
            "{message}\n\nContext/output limit: the provider rejected the request because the prompt plus requested output appears to exceed a model limit. Try reducing prompt history, lowering max output tokens, or enabling/prioritizing context pruning before retrying."
        ));
    }

    // Class-specific, actionable guidance for terminal failures (#931). After
    // retries are exhausted (or for a non-retryable error), name the class and
    // the remediation rather than surfacing a raw provider string.
    if let Some(guidance) = terminal_class_guidance(&DomainError::Provider(message.clone())) {
        return DomainError::Provider(format!("{message}\n\n{guidance}"));
    }

    DomainError::Provider(message)
}

/// Class-specific remediation appended to a terminal provider error so the
/// agent/parent gets an actionable message (e.g. "rate limited … try later")
/// instead of a raw string. `Auth` gets a re-authenticate hint (no retry-later);
/// `Client` (4xx) is the request's own fault and is passed through unchanged.
fn terminal_class_guidance(err: &DomainError) -> Option<&'static str> {
    match classify_provider_error(err) {
        ProviderErrorClass::RateLimit => Some(
            "Rate limit: the provider throttled the request. It was retried with backoff and still failed — wait and retry later, or reduce request frequency.",
        ),
        ProviderErrorClass::Server => Some(
            "Server/overload: the provider is overloaded or returned a 5xx error. It was retried and still failed — retry later.",
        ),
        ProviderErrorClass::Network => Some(
            "Network: could not reach the provider (connection/timeout). It was retried and still failed — check connectivity and retry later.",
        ),
        ProviderErrorClass::Auth => Some(
            "Authentication: the provider rejected your credentials (not retryable). Check the API key or re-authenticate (`quecto auth login`), then retry.",
        ),
        ProviderErrorClass::Client
        | ProviderErrorClass::Cancelled
        | ProviderErrorClass::Unknown => None,
    }
}

pub(super) fn is_context_or_output_limit_error(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    (lowered.contains("maximum context length")
        || lowered.contains("context length")
        || lowered.contains("context window")
        || lowered.contains("too many tokens")
        || lowered.contains("max_tokens")
        || lowered.contains("max output")
        || lowered.contains("requested") && lowered.contains("tokens"))
        && (lowered.contains("token") || lowered.contains("context"))
}
