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
    let text = format!("HTTP {status} from {vendor}: class={class}; refusal={refusal}");
    assert!(text.len() <= BODY_LIMIT);
    text
}

/// The HTTP formatter and receipt share the same accepted error envelope.
pub(super) fn http_error_envelope(value: &serde_json::Value) -> &serde_json::Value {
    value
        .get("error")
        .filter(|v| v.is_object())
        .unwrap_or(value)
}

pub(super) fn raw_json(raw: &str) -> Option<serde_json::Value> {
    match raw.len() <= BODY_LIMIT {
        true => serde_json::from_str(raw).ok(),
        false => None,
    }
}

pub(super) fn http(status: u16, vendor: &str, body: &str) -> String {
    let value = raw_json(body);
    let mut class = ProviderErrorClass::from_status(status);
    let mut refusal = false;
    if let Some(value) = value.as_ref().filter(|v| v.is_object()) {
        let error = http_error_envelope(value);
        let fields = [error["type"].as_str(), error["code"].as_str()];
        if fields.into_iter().flatten().any(is_billing_error_name) {
            class = ProviderErrorClass::Billing;
        } else if class == ProviderErrorClass::Server
            && fields.contains(&Some("invalid_request_error"))
        {
            class = ProviderErrorClass::Client;
        }
        refusal = class == ProviderErrorClass::Client
            && refusal_compatible(error)
            && match (vendor, status) {
                ("OpenAI", 404) => fields.contains(&Some("model_not_found")),
                ("Anthropic", 404) => {
                    fields.contains(&Some("not_found_error"))
                        && error["message"]
                            .as_str()
                            .is_some_and(|m| m.starts_with("model:"))
                }
                ("Codex", 400) => value["detail"]
                    .as_str()
                    .or_else(|| error["message"].as_str())
                    .is_some_and(|m| {
                        m.contains("model is not supported when using Codex with a ChatGPT account")
                    }),
                _ => false,
            };
    }
    let marker = if refusal {
        "model"
    } else if vendor == "Codex"
        && status == 400
        && value.as_ref().is_some_and(|v| {
            v["error"]["code"].as_str() == Some("invalid_encrypted_content")
                && refusal_compatible(&v["error"])
        })
    {
        "encrypted"
    } else {
        "none"
    };
    let marker = match value.as_ref() {
        Some(value)
            if class == ProviderErrorClass::Client
                && matches!(status, 400 | 422 | 500..=599)
                && marker == "none"
                && is_context_limit(value.get("error").unwrap_or(value)) =>
        {
            "none; limit=context length"
        }
        _ => marker,
    };
    render(status, vendor, class, marker)
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
    match raw_json(raw) {
        Some(value) => stream(vendor, &value),
        None => render(502, vendor, ProviderErrorClass::Server, "none"),
    }
}

/// Contradictory known auth, billing or server declarations cannot establish
/// refusal authority. Unknown fields are inert (the known vendor shape wins).
fn refusal_compatible(error: &serde_json::Value) -> bool {
    [error["type"].as_str(), error["code"].as_str()]
        .into_iter()
        .flatten()
        .all(super::attempt_profile::refusal_compatible_declaration)
}

fn is_context_limit(error: &serde_json::Value) -> bool {
    [error["type"].as_str(), error["code"].as_str()].contains(&Some("context_length_exceeded"))
}

pub(super) fn stream(vendor: &str, value: &serde_json::Value) -> String {
    assert!(["OpenAI", "Codex", "Anthropic"].contains(&vendor));
    let (status, marker) = match serde_json::to_writer(&mut Budget(0), value).is_ok() {
        true => {
            if value["type"].as_str() == Some("response.incomplete")
                && value["response"]["error"].is_null()
            {
                return format!("{vendor} stream incomplete");
            }
            let error = match value["type"].as_str() {
                Some("response.failed" | "response.incomplete") => &value["response"]["error"],
                _ => value
                    .get("error")
                    .filter(|v| v.is_object())
                    .unwrap_or(value),
            };
            // Reuse the typed protocol allowlist, never arbitrary message text.
            let normalized = serde_json::json!({"error": error});
            (
                super::attempt_profile::stream_error_status(&normalized),
                if is_context_limit(error) {
                    "none; limit=context length"
                } else {
                    "none"
                },
            )
        }
        false => (502, "none"),
    };
    render(
        status,
        vendor,
        ProviderErrorClass::from_status(status),
        marker,
    )
}

#[cfg(test)]
#[path = "provider_diagnostics_tests.rs"]
mod tests;
