//! The only payload-to-diagnostic boundary. Parse at most 4096 bytes,
//! keep no wire prose, and emit only domain-approved finite markers.
use super::stream_idle::{BodyError, StreamIdle};
use crate::domain::inference::services::provider_error::{
    ProviderErrorClass, is_billing_error_name,
};

pub(super) const BODY_LIMIT: usize = 4096;

/// Stop reading as soon as the diagnostic budget is exceeded. No prefix of
/// an oversized or invalid UTF-8 body may establish semantic authority.
pub(super) async fn read(
    response: reqwest::Response,
    idle: StreamIdle,
) -> Result<String, BodyError> {
    read_bounded(response, Some(idle)).await
}

/// A whole chat exchange already has its total bound; preserve that policy.
pub(super) async fn read_reply(response: reqwest::Response) -> Result<String, BodyError> {
    read_bounded(response, None).await
}

async fn read_bounded(
    mut response: reqwest::Response,
    idle: Option<StreamIdle>,
) -> Result<String, BodyError> {
    let mut bytes = Vec::with_capacity(BODY_LIMIT);
    loop {
        let next = match idle {
            Some(idle) => idle.within(response.chunk()).await,
            None => Ok(response.chunk().await),
        };
        match next {
            Ok(Ok(Some(chunk))) if bytes.len().saturating_add(chunk.len()) <= BODY_LIMIT => {
                bytes.extend_from_slice(&chunk)
            }
            Ok(Ok(Some(_))) => return Ok(String::new()),
            Ok(Ok(None)) => return Ok(String::from_utf8(bytes).unwrap_or_default()),
            Ok(Err(error)) => return Err(BodyError::Read(error)),
            Err(error) => return Err(BodyError::Idle(error)),
        }
        assert!(bytes.len() <= BODY_LIMIT);
    }
}

fn render(status: u16, vendor: &str, class: ProviderErrorClass, refusal: &str) -> String {
    assert!(["OpenAI", "Codex", "Anthropic"].contains(&vendor));
    assert!(["model", "none", "encrypted", "none; limit=context length"].contains(&refusal));
    assert!((100..=999).contains(&status));
    use std::fmt::Write;
    let mut text = String::with_capacity(128);
    write!(
        &mut text,
        "HTTP {status} from {vendor}: class={class}; refusal={refusal}"
    )
    .expect("writing finite diagnostic");
    assert!(text.len() <= 128);
    assert_eq!(text.capacity(), 128);
    assert!(text.len() <= BODY_LIMIT);
    text
}

pub(super) fn packet(raw: &str) -> Option<super::error_fields::Packet> {
    (raw.len() <= BODY_LIMIT)
        .then(|| super::error_fields::parse(raw).ok())
        .flatten()
}
pub(super) fn http(status: u16, vendor: &str, body: &str) -> String {
    let value = packet(body).unwrap_or_default();
    let error = value.http();
    let fields = error.names();
    let mut class = ProviderErrorClass::from_status(status);
    if fields.into_iter().flatten().any(is_billing_error_name) {
        class = ProviderErrorClass::Billing;
    } else if class == ProviderErrorClass::Server && fields.contains(&Some("invalid_request_error"))
    {
        class = ProviderErrorClass::Client;
    }
    let root_selected = matches!(value.fields[1].kind, 0 | 2 | 3 | 4)
        && matches!(value.fields[2].kind, 0 | 2 | 3 | 4);
    let refusal = class == ProviderErrorClass::Client
        && compatible(error)
        && match (vendor, status) {
            ("OpenAI", 404) => error.has("model_not_found"),
            ("Anthropic", 404) => error.has("not_found_error") && error.message_model,
            ("Codex", 400) => {
                if root_selected {
                    error.detail_refusal || error.message_refusal
                } else {
                    error.message_refusal
                }
            }
            _ => false,
        };
    let marker = if refusal {
        "model"
    } else if vendor == "Codex"
        && status == 400
        && error.has("invalid_encrypted_content")
        && compatible(error)
    {
        "encrypted"
    } else if class == ProviderErrorClass::Client
        && matches!(status, 400 | 422 | 500..=599)
        && error.has("context_length_exceeded")
    {
        "none; limit=context length"
    } else {
        "none"
    };
    render(status, vendor, class, marker)
}
fn compatible(error: super::error_fields::Fields) -> bool {
    error
        .names()
        .into_iter()
        .flatten()
        .all(super::attempt_profile::refusal_compatible_declaration)
}

/// Count serialization without allocating another copy of an SSE payload.
struct Budget(usize);
impl std::io::Write for Budget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        match self.0 <= BODY_LIMIT {
            true => Ok(bytes.len()),
            false => Err(std::io::Error::other("diagnostic budget exceeded")),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn stream_raw(vendor: &str, raw: &str) -> String {
    // Legacy assembled framing may trim Unicode whitespace. Only the valid
    // bounded RAW JSON, not that normalized value, grants error semantics.
    match packet(raw) {
        Some(value) => stream_packet(vendor, value),
        None => render(502, vendor, ProviderErrorClass::Server, "none"),
    }
}
pub(super) fn stream_packet(vendor: &str, value: super::error_fields::Packet) -> String {
    if value.event() == Some("response.incomplete") && value.fields[2].kind == 0 {
        return format!("{vendor} stream incomplete");
    }
    stream_fields(vendor, value.stream())
}
fn stream_fields(vendor: &str, error: super::error_fields::Fields) -> String {
    let status = super::attempt_profile::status_fields(error);
    render(
        status,
        vendor,
        ProviderErrorClass::from_status(status),
        if error.has("context_length_exceeded") {
            "none; limit=context length"
        } else {
            "none"
        },
    )
}

/// Exact consumer-selected SSE envelope, not an aggregate of declarations.
/// Responses failure/incomplete events own response.error; other supported
/// errors use the direct object or root. An empty selection still shadows
/// ignored envelopes, which cannot grant or suppress typed authority.
pub(super) fn stream_error_envelope(value: &serde_json::Value) -> &serde_json::Value {
    match value["type"].as_str() {
        Some("response.failed" | "response.incomplete") => &value["response"]["error"],
        _ => value
            .get("error")
            .filter(|v| v.is_object())
            .unwrap_or(value),
    }
}

pub(super) fn stream_throttle(value: &serde_json::Value) -> bool {
    matches!(
        super::attempt_profile::status_fields(super::error_fields::value_fields(
            stream_error_envelope(value)
        )),
        429 | 529
    )
}

pub(super) fn stream(vendor: &str, value: &serde_json::Value) -> String {
    if serde_json::to_writer(&mut Budget(0), value).is_ok() {
        if value["type"].as_str() == Some("response.incomplete")
            && value["response"]["error"].is_null()
        {
            return format!("{vendor} stream incomplete");
        }
        stream_fields(
            vendor,
            super::error_fields::value_fields(stream_error_envelope(value)),
        )
    } else {
        render(502, vendor, ProviderErrorClass::Server, "none")
    }
}

#[cfg(test)]
#[path = "provider_diagnostics_tests.rs"]
mod tests;
