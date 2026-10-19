//! What to tell the agent when a provider request ends the turn (#931,
//! #2459): name the failure's class and a remediation that matches what
//! happened. A retry is claimed only when one was made, and "could not
//! reach the provider" only when no response arrived.
use crate::domain::inference::services::provider_error::{
    ProviderErrorClass, classify_provider_error, is_admission_gate_error,
    is_context_or_output_limit_error, model_refusal,
};
use crate::domain::shared::error::DomainError;

/// Whether a failed request had already shown output to the user (#2155).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// Nothing reached the user: the request may be sent again.
    NotShown,
    /// Text, thinking or a tool call already streamed: sending the request
    /// again would show a second reply after the first one's fragment.
    Shown,
}

impl Output {
    /// The output state of a stream that did (`true`) or did not emit events.
    pub fn from_emitted(emitted_event: bool) -> Self {
        match emitted_event {
            true => Self::Shown,
            false => Self::NotShown,
        }
    }
}

/// Whether any attempt of the request got a response from the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    /// An attempt got an HTTP status or streamed output.
    Received,
    /// Every attempt left a record and none got a status: a connect error
    /// or a timeout before any response.
    NotReceived,
    /// The attempts left no record to tell.
    Unknown,
}

/// What a failed request did: the facts the guidance is chosen by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptFacts {
    /// Attempts made, the first included (zero reads as one).
    pub attempts: u32,
    pub output: Output,
    pub response: Response,
}

/// How the guidance for a model refused for the account or auth mode ends
/// (#2435). The loop does not know the hold, so the guidance says what a
/// hold does without claiming one: with `model_refusal_ttl_secs = 0` none is.
const MODEL_REFUSED_GUIDANCE: &str = "Model unavailable: the provider refused this model for \
     the account or auth mode in use. While a refusal is held (providers.model_refusal_ttl_secs, \
     an hour by default; 0 holds none) the model is not offered, until the provider serves it \
     again; choose another model (list_models, then set_model).";

/// Why a reply that already streamed output was not sent again.
const NOT_REPLAYED: &str = "It was not retried, because a reply that has already streamed \
     output is never replayed;";

/// The guidance a prompt plus requested output over a model limit gets.
const CONTEXT_LIMIT_GUIDANCE: &str = "Context/output limit: the provider rejected the request because the prompt plus requested output appears to exceed a model limit. Try reducing prompt history, lowering max output tokens, or enabling/prioritizing context pruning before retrying.";

/// `err` with the guidance its class and `facts` call for appended, once.
/// Anything but a provider error, and a class with nothing to add, is
/// returned as it is.
pub fn explain_terminal_failure(err: DomainError, facts: AttemptFacts) -> DomainError {
    let DomainError::Provider(message) = err else {
        return err;
    };
    match guidance_for(&message, facts) {
        Some(guidance) => DomainError::Provider(format!("{message}\n\n{guidance}")),
        None => DomainError::Provider(message),
    }
}

/// The guidance `message` gets: the inference-admission gate's error says
/// what to do itself; a limit or a refusal names its own remediation (each
/// appended once); any other failure gets its class's.
fn guidance_for(message: &str, facts: AttemptFacts) -> Option<String> {
    let err = DomainError::Provider(message.to_string());
    if is_admission_gate_error(&err) {
        return None;
    }
    if is_context_or_output_limit_error(message) {
        let explained = message
            .to_ascii_lowercase()
            .contains("context/output limit");
        return (!explained).then(|| CONTEXT_LIMIT_GUIDANCE.to_string());
    }
    if model_refusal(&err).is_some() {
        return (!message.ends_with(MODEL_REFUSED_GUIDANCE))
            .then(|| MODEL_REFUSED_GUIDANCE.to_string());
    }
    class_guidance(&classify_provider_error(&err), facts)
}

/// The remediation for a terminal failure of `class`. `Client` (the
/// request's own fault), `Cancelled`, `Admission` (its refusal already says
/// what to do) and `Unknown` add none.
fn class_guidance(class: &ProviderErrorClass, facts: AttemptFacts) -> Option<String> {
    let retried = facts.attempts > 1;
    let n = facts.attempts;
    let shown = facts.output == Output::Shown;
    Some(match class {
        ProviderErrorClass::RateLimit if shown => format!(
            "Rate limit: the provider throttled the request partway through the reply. {NOT_REPLAYED} continuing or re-prompting is safe once the limit clears."
        ),
        ProviderErrorClass::RateLimit if retried => format!(
            "Rate limit: the provider throttled the request. It was retried with backoff ({n} attempts) and still failed — wait and retry later, or reduce request frequency."
        ),
        ProviderErrorClass::RateLimit => "Rate limit: the provider throttled the request. It was not retried — wait and retry later, or reduce request frequency.".into(),
        ProviderErrorClass::Server if shown => format!(
            "Server/overload: the provider failed partway through the reply (overloaded or a 5xx error). {NOT_REPLAYED} continuing or re-prompting is safe."
        ),
        ProviderErrorClass::Server if retried => format!(
            "Server/overload: the provider is overloaded or returned a 5xx error. It was retried ({n} attempts) and still failed — retry later."
        ),
        ProviderErrorClass::Server => "Server/overload: the provider is overloaded or returned a 5xx error. It was not retried — retry later.".into(),
        ProviderErrorClass::Network if shown => format!(
            "Network: the provider closed the connection partway through the reply. {NOT_REPLAYED} continuing or re-prompting is safe."
        ),
        ProviderErrorClass::Network => network_guidance(facts),
        ProviderErrorClass::EmptyStream => "Empty stream: the provider stream ended without assistant output. This is a synthetic classification; no HTTP failure or overload is established.".into(),
        ProviderErrorClass::Stalled => "Stalled: the provider stopped sending, or kept sending events with no output (the limit is in the error above), and the reply was abandoned. Retry later.".into(),
        ProviderErrorClass::OutputCapped => "Output cap: the reply kept streaming past its output cap (the cap is in the error above), the mark of a runaway such as a repetition loop, and was abandoned. It was not retried: resending the same request tends to repeat it. Narrow or rephrase the request before trying again.".into(),
        ProviderErrorClass::Auth => "Authentication: the provider rejected your credentials (not retryable). Check the API key or re-authenticate (`quecto auth login`), then retry.".into(),
        ProviderErrorClass::Billing => "Billing/quota: the provider reports exhausted credit or a billing limit (not retryable). Check the account plan, billing, or quota before retrying.".into(),
        ProviderErrorClass::Client
        | ProviderErrorClass::Cancelled
        | ProviderErrorClass::Admission
        | ProviderErrorClass::Unknown => return None,
    })
}

/// A network failure before any output: "could not reach" only when no
/// attempt got a response.
fn network_guidance(facts: AttemptFacts) -> String {
    let n = facts.attempts;
    match (facts.response, facts.attempts > 1) {
        (Response::NotReceived, true) => format!(
            "Network: could not reach the provider (connection/timeout). It was retried ({n} attempts) and still failed — check connectivity and retry later."
        ),
        (Response::NotReceived, false) => "Network: could not reach the provider (connection/timeout). It was not retried — check connectivity and retry later.".into(),
        (Response::Received | Response::Unknown, true) => format!(
            "Network: the connection to the provider failed (connection/timeout). It was retried ({n} attempts) and still failed — retry later."
        ),
        (Response::Received | Response::Unknown, false) => "Network: the connection to the provider failed (connection/timeout). It was not retried — retry later.".into(),
    }
}

#[cfg(test)]
#[path = "terminal_guidance_tests.rs"]
mod tests;
