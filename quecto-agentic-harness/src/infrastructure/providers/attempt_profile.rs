//! Existing wire/error behavior is a property of a provider and call surface,
//! not admission policy. Keep these compatibility decisions out of ownership.
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
}
impl Profile {
    pub const fn new(vendor: Vendor, surface: Surface) -> Self {
        Self { vendor, surface }
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
