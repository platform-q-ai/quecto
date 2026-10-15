//! Finite error authority. No owned provider strings, trees or decoded scratch.
#[path = "error_fields_scan.rs"]
mod scan;
#[path = "attempt_error_string.rs"]
mod strings;
use strings::Token;

pub(super) const SCRATCH_BOUND: usize = 1664;
const _: () = assert!(SCRATCH_BOUND <= 4096);
const NAMES: &[&str] = &[
    "",
    "error",
    "response.failed",
    "response.incomplete",
    "response.completed",
    "authentication_error",
    "invalid_api_key",
    "permission_error",
    "not_found_error",
    "model_not_found",
    "request_too_large",
    "context_length_exceeded",
    "invalid_request_error",
    "server_error",
    "api_error",
    "internal_error",
    "internal_server_error",
    "service_unavailable",
    "service_unavailable_error",
    "timeout_error",
    "rate_limit_error",
    "rate_limit_exceeded",
    "overloaded_error",
    "insufficient_quota",
    "usage_limit_reached",
    "billing_error",
    "billing_hard_limit_reached",
    "invalid_encrypted_content",
    "max_output_tokens",
    "content_filter",
];
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Fields {
    pub kind: u8,
    pub name: u8,
    pub code: u8,
    pub numeric: u16,
    pub message_model: bool,
    pub message_refusal: bool,
    pub detail_refusal: bool,
}
impl Fields {
    pub fn names(self) -> [Option<&'static str>; 2] {
        [self.name, self.code].map(|id| {
            NAMES
                .get(usize::from(id))
                .copied()
                .filter(|s| !s.is_empty())
        })
    }
    pub fn has(self, name: &str) -> bool {
        self.names().contains(&Some(name))
    }
}
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Packet {
    pub fields: [Fields; 3],
    pub reason: u8,
    pub object: bool,
}
impl Packet {
    pub fn event(self) -> Option<&'static str> {
        self.fields[0].names()[0]
    }
    pub fn stream(self) -> Fields {
        match self.event() {
            Some("response.failed" | "response.incomplete") => self.fields[2],
            _ if self.fields[1].kind == 1 => self.fields[1],
            _ => self.fields[0],
        }
    }
    pub fn http(self) -> Fields {
        if self.fields[1].kind == 1 {
            self.fields[1]
        } else if self.fields[2].kind == 1 {
            self.fields[2]
        } else {
            self.fields[0]
        }
    }
    pub fn failure(
        self,
        vendor: super::attempt_profile::Vendor,
    ) -> Option<super::codex_event::TerminalEvent> {
        use super::attempt_profile::Vendor;
        use super::codex_event::TerminalEvent as T;
        if self.object {
            match vendor {
                Vendor::OpenAi if matches!(self.fields[1].kind, 1 | 2) => Some(T::Error),
                Vendor::Codex => match self.event() {
                    Some("error") => Some(T::Error),
                    Some("response.failed") => Some(T::ResponseFailed),
                    Some("response.incomplete") => Some(T::ResponseIncomplete),
                    None if matches!(self.fields[1].kind, 1 | 2) && self.fields[0].kind != 2 => {
                        Some(T::Error)
                    }
                    _ => None,
                },
                _ => None,
            }
        } else {
            None
        }
    }
}
fn symbol(token: &Token) -> u8 {
    NAMES
        .iter()
        .position(|name| token.is(name))
        .and_then(|id| u8::try_from(id).ok())
        .unwrap_or(0)
}
pub(super) fn parse(raw: &str) -> Result<Packet, serde_json::Error> {
    scan::parse(raw).map_err(|()| serde::de::Error::custom("invalid error event JSON"))
}
const _: () = assert!(std::mem::size_of::<Fields>() <= 32);
const _: () = assert!(std::mem::size_of::<Packet>() <= 128);

pub(super) fn value_fields(value: &serde_json::Value) -> Fields {
    let id = |key: &str| {
        value[key]
            .as_str()
            .and_then(|s| NAMES.iter().position(|name| *name == s))
            .map(|n| n as u8)
            .unwrap_or(0)
    };
    Fields {
        kind: if value.is_object() { 1 } else { 0 },
        name: id("type"),
        code: id("code"),
        numeric: value["code"]
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .unwrap_or(0),
        message_model: value["message"]
            .as_str()
            .is_some_and(|s| s.starts_with("model:")),
        message_refusal: value["message"].as_str().is_some_and(|s| {
            s.contains("model is not supported when using Codex with a ChatGPT account")
        }),
        detail_refusal: value["detail"].as_str().is_some_and(|s| {
            s.contains("model is not supported when using Codex with a ChatGPT account")
        }),
    }
}
pub(super) fn throttle(fields: Fields) -> bool {
    let names = fields.names();
    if names
        .into_iter()
        .flatten()
        .any(crate::domain::inference::services::provider_error::is_billing_error_name)
    {
        false
    } else {
        names.into_iter().flatten().any(|n| {
            matches!(
                n,
                "rate_limit_error" | "rate_limit_exceeded" | "overloaded_error"
            )
        })
    }
}
pub(super) fn reason(
    id: u8,
) -> Option<crate::domain::inference::value_objects::attempt_diagnostics::IncompleteReason> {
    use crate::domain::inference::value_objects::attempt_diagnostics::IncompleteReason as R;
    match NAMES.get(usize::from(id)).copied() {
        Some("max_output_tokens") => Some(R::MaxOutputTokens),
        Some("content_filter") => Some(R::ContentFilter),
        _ => None,
    }
}
#[cfg(test)]
#[path = "error_fields_tests.rs"]
mod tests;
