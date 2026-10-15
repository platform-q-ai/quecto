//! Existing wire/error behavior is a property of a provider and call surface,
//! not admission policy. Keep these compatibility decisions out of ownership.
use super::stream_idle::{Idle, StreamIdle};
use crate::domain::error::DomainError;

#[derive(Clone, Copy)]
pub(super) enum Vendor {
    OpenAi,
    Codex,
    Anthropic,
}
#[derive(Clone, Copy)]
pub(super) enum Surface {
    Chat,
    Assembled,
    Incremental,
}
#[derive(Clone, Copy)]
pub(super) struct Profile {
    pub vendor: Vendor,
    pub surface: Surface,
    /// The provider's bound on a silent stream (#2210).
    pub idle: StreamIdle,
}
impl Profile {
    /// A profile bound by `idle`: always the provider's own bounds, which
    /// every call site must name (#2210 review), so none can fall back to
    /// the defaults and ignore a provider's chosen bounds.
    pub fn new(vendor: Vendor, surface: Surface, idle: StreamIdle) -> Self {
        Self {
            vendor,
            surface,
            idle,
        }
    }
    /// Whether the reply streams, so its send and every read of its body are
    /// bounded by [`Self::idle`]. A whole non-streaming reply sends nothing
    /// until it is complete: there the bound would be a total one.
    pub fn streams(self) -> bool {
        matches!(self.surface, Surface::Assembled | Surface::Incremental)
    }
    /// Await one step of the exchange: bounded when the reply streams.
    pub async fn within<F: std::future::Future>(self, step: F) -> Result<F::Output, Idle> {
        match self.streams() {
            true => self.idle.within(step).await,
            false => Ok(step.await),
        }
    }
    /// The idle bound of this reply's SSE body, measured from its last event
    /// (#2433). Only a streaming reply has an SSE body read this way.
    pub fn event_idle(self) -> super::stream_idle::EventIdle {
        assert!(self.streams(), "only a streaming reply has an SSE body");
        super::stream_idle::EventIdle::events(self.idle)
    }
    pub fn name(self) -> &'static str {
        match self.vendor {
            Vendor::OpenAi => "OpenAI",
            Vendor::Codex => "Codex",
            Vendor::Anthropic => "Anthropic",
        }
    }
    pub fn send_error(self, error: &reqwest::Error) -> DomainError {
        let message = match self.vendor {
            Vendor::Anthropic => format!("HTTP error: {error}"),
            Vendor::Codex => format!(
                "Codex request failed: {}",
                super::sse_common::format_send_error(error)
            ),
            Vendor::OpenAi => format!(
                "HTTP error: {}",
                super::sse_common::format_send_error(error)
            ),
        };
        DomainError::Provider(message)
    }
    pub fn read_error(self, error: &reqwest::Error) -> DomainError {
        let label = if matches!(
            (self.vendor, self.surface),
            (Vendor::Anthropic, Surface::Assembled)
        ) {
            "stream"
        } else {
            "response"
        };
        DomainError::Provider(format!("failed to read {label}: {error}"))
    }
    pub fn strict_error_body(self) -> bool {
        matches!(
            (self.vendor, self.surface),
            (Vendor::OpenAi | Vendor::Anthropic, Surface::Chat)
        )
    }
}

/// Known client-side error types and codes (OpenAI, Anthropic, OpenRouter
/// and compatible vendors), in precedence order, with the HTTP status each
/// stands for: the provider refused the request for what it is, so resending
/// it unchanged fails again. A billing error, the domain's list, precedes
/// them all as [`BILLING_STATUS`].
const CLIENT_ERRORS: &[(&str, u16)] = &[
    ("authentication_error", 401),
    ("invalid_api_key", 401),
    ("permission_error", 403),
    ("not_found_error", 404),
    ("model_not_found", 404),
    ("request_too_large", 413),
    ("context_length_exceeded", 400),
    ("invalid_request_error", 400),
];
/// The status a billing error stands for: payment required.
const BILLING_STATUS: u16 = 402;
/// Known server-side error types and codes, with the HTTP status each
/// stands for: the provider failed, and a later attempt may succeed.
const SERVER_ERRORS: &[(&str, u16)] = &[
    ("server_error", 500),
    ("api_error", 500),
    ("internal_error", 500),
    ("internal_server_error", 500),
    ("service_unavailable", 503),
    ("service_unavailable_error", 503),
    ("timeout_error", 504),
];
/// An error chunk nobody knows: the provider's side failed in a way it did
/// not name. Retryable, like a gateway failure, never a client error.
const UNKNOWN_ERROR_STATUS: u16 = 502;
/// A request timeout (408, OpenRouter's upstream timeout) stands for a
/// gateway timeout: the provider did not answer in time, and a later
/// attempt may.
const REQUEST_TIMEOUT: u16 = 408;
const GATEWAY_TIMEOUT: u16 = 504;

/// Whether an OpenAI-compatible error chunk is a throttle the admission gate
/// should hear of (#2155 review): its status is 429 or 529, typed or numeric
/// (OpenRouter's `{"code":429}`). A billing error is never one: it is 402
/// before any throttle or numeric status is considered.
pub(super) fn is_throttle_chunk(value: &serde_json::Value) -> bool {
    matches!(stream_error_status(value), 429 | 529)
}

/// Whether an OpenAI-compatible SSE chunk is a mid-stream error chunk
/// (#2236): its `error` is an object of any shape (`{"error":{...}}`,
/// OpenAI's; `{}` names no cause, so it is the unknown, retryable 502,
/// #2249 review) or a non-empty string (`{"error":"..."}`, Ollama's native
/// API and some OpenAI-compatible servers). Only these shapes are allowed;
/// an `error` of any other shape (null, a number, a list, `""`) says
/// nothing failed and is not one. Both the stream handler and the attempt
/// observer ask this one question, so a reply cut short is an error on
/// every path, with admission or without.
pub(super) fn is_stream_error_chunk(value: &serde_json::Value) -> bool {
    match value.get("error") {
        Some(serde_json::Value::Object(_)) => true,
        Some(serde_json::Value::String(text)) => !text.is_empty(),
        _ => false,
    }
}

/// Whether a Responses (Codex) SSE chunk is an untyped error chunk
/// (#2249 review): it has no string `type` (none, or `null`) and is an
/// error chunk by [`is_stream_error_chunk`] — a gateway's or a compatible
/// server's shape. A typed event keeps the meaning its type gives it.
pub(super) fn is_untyped_error_chunk(value: &serde_json::Value) -> bool {
    value["type"].as_str().is_none() && is_stream_error_chunk(value)
}

/// An OpenAI-compatible mid-stream `data: {"error":{...}}` chunk as the
/// stream's terminal error (#2155), rendered with the HTTP status its typed
/// fields stand for, so the retry classifier reads it as the provider meant
/// it. Status is derived from typed protocol fields, never message text:
///
/// 1. a billing error type or code is 402 (never retried);
/// 2. an allowlisted client error type or code is its 4xx (never retried);
/// 3. a typed throttle is 429, or 529 when overloaded;
/// 4. a known server error type or code is its 5xx (retried);
/// 5. a numeric `error.code` (OpenRouter's shape) is that status when the
///    retry classifier knows it; a 408 is a gateway timeout (504) and any
///    other 5xx (Cloudflare's 520-524) a bad gateway (502), both retried;
///    another 4xx stands as itself (never retried);
/// 6. anything else is 502: retryable. A bare string `error` (#2236)
///    carries no typed fields, so it is always this unknown 502.
///
/// Typed fields always win over a numeric code: a named failure says more
/// than a status a gateway may have wrapped it in (a client type in a 5xx,
/// #935, stays a client error).
///
/// The chunk ends the attempt: the text already streamed stays streamed (it
/// was shown once, and the retry owner never replays a reply after output),
/// and no completed reply follows the error, so a reply cut short by a
/// provider failure is never taken as a whole answer.
#[cfg(test)]
pub(super) fn openai_stream_error(value: &serde_json::Value) -> String {
    assert!(
        is_stream_error_chunk(value),
        "only an error chunk renders as a stream error"
    );
    let status = stream_error_status(value);
    assert!(
        (400..=599).contains(&status),
        "an error chunk always renders an HTTP error status"
    );
    super::provider_diagnostics::stream("OpenAI", value)
}

/// Unknown declarations are inert; each recognized declaration must itself
/// describe a compatible client refusal, never an outage or throttle.
pub(super) fn refusal_compatible_declaration(name: &str) -> bool {
    let known = CLIENT_ERRORS
        .iter()
        .chain(SERVER_ERRORS)
        .find(|(n, _)| *n == name)
        .map(|(_, status)| *status)
        .or_else(|| {
            match (
                crate::domain::inference::services::provider_error::is_billing_error_name(name),
                matches!(
                    name,
                    "rate_limit_error" | "overloaded_error" | "rate_limit_exceeded"
                ),
            ) {
                (true, _) => Some(BILLING_STATUS),
                (_, true) => Some(429),
                _ => None,
            }
        });
    matches!(known, None | Some(400 | 404))
}

/// The HTTP status an OpenAI-compatible error chunk stands for; see
/// [`openai_stream_error`].
pub(super) fn stream_error_status(value: &serde_json::Value) -> u16 {
    status_fields(super::error_fields::value_fields(&value["error"]))
}
pub(super) fn status_fields(error: super::error_fields::Fields) -> u16 {
    use crate::domain::inference::services::provider_error::{
        ProviderErrorClass, is_billing_error_name,
    };
    let fields = error.names();
    if fields.into_iter().flatten().any(is_billing_error_name) {
        return BILLING_STATUS;
    }
    let named = |table: &[(&str, u16)]| {
        table
            .iter()
            .find(|(name, _)| fields.contains(&Some(*name)))
            .map(|(_, status)| *status)
    };
    if let Some(status) = named(CLIENT_ERRORS) {
        return status;
    }
    if super::error_fields::throttle(error) {
        return if fields.contains(&Some("overloaded_error")) {
            529
        } else {
            429
        };
    }
    if let Some(status) = named(SERVER_ERRORS) {
        return status;
    }
    let numeric = (400..=599)
        .contains(&error.numeric)
        .then_some(error.numeric);
    match numeric {
        Some(status) if ProviderErrorClass::from_status(status) != ProviderErrorClass::Unknown => {
            status
        }
        Some(REQUEST_TIMEOUT) => GATEWAY_TIMEOUT,
        Some(status @ 400..=499) => status,
        _ => UNKNOWN_ERROR_STATUS,
    }
}

#[cfg(test)]
#[path = "attempt_profile_tests.rs"]
mod tests;
