//! Existing wire/error behavior is a property of a provider and call surface,
//! not admission policy. Keep these compatibility decisions out of ownership.
use super::stream_idle::{BodyError, Idle, StreamIdle};
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
    /// Read a whole body: each read bounded when the reply streams.
    pub async fn text(self, response: reqwest::Response) -> Result<String, BodyError> {
        match self.streams() {
            true => self.idle.text(response).await,
            false => response.text().await.map_err(BodyError::Read),
        }
    }
    pub fn name(self) -> &'static str {
        match self.vendor {
            Vendor::OpenAi => "OpenAI",
            Vendor::Codex => "Codex",
            Vendor::Anthropic => "Anthropic",
        }
    }
    pub fn openai(self) -> bool {
        matches!(self.vendor, Vendor::OpenAi)
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
    pub fn suffix(self, headers: &reqwest::header::HeaderMap) -> String {
        if matches!(self.vendor, Vendor::Codex) {
            String::new()
        } else {
            super::sse_common::retry_after_suffix(headers)
        }
    }
    pub fn error_body(self, body: String) -> String {
        if matches!(self.surface, Surface::Incremental) {
            super::sse_common::truncate_error_body(body)
        } else {
            body
        }
    }
}

/// New enabled OpenAI SSE errors retain machine fields for the existing retry
/// classifier. Status is derived from typed protocol fields, never message text.
pub(super) fn openai_stream_error(value: &serde_json::Value) -> String {
    let error = &value["error"];
    let fields = [error["type"].as_str(), error["code"].as_str()];
    let status = if fields.iter().any(|field| {
        matches!(
            field,
            Some("authentication_error" | "permission_error" | "invalid_api_key")
        )
    }) {
        401
    } else if fields.contains(&Some("invalid_request_error")) {
        400
    } else if super::admission_feedback::is_typed_throttle(value) {
        if fields.contains(&Some("overloaded_error")) {
            529
        } else {
            429
        }
    } else {
        400
    };
    format!("HTTP {status} OpenAI stream error: {value}")
}

#[cfg(test)]
#[path = "attempt_profile_tests.rs"]
mod tests;
