//! Terminal provider-error enhancement for the agent loop (#931).
//!
//! After retries are exhausted (or for a non-retryable error), turn a raw
//! provider string into a classified, actionable message: name the error class
//! and the remediation so the agent/parent can react sensibly.

use crate::domain::error::DomainError;
use crate::domain::provider_error::{ProviderErrorClass, classify_provider_error};

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
/// instead of a raw string. `Client` (4xx) is the request's own fault and is
/// passed through without retry-later advice.
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
        ProviderErrorClass::Auth
        | ProviderErrorClass::Client
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
