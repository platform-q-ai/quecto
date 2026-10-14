//! One dispatch decision for Codex whole/incremental consumers and receipts.
//! Dispatch follows existing surface acceptance. Raw semantic admission remains
//! separate: neither trimming nor lossy byte repair grants typed authority.
use super::attempt_profile::{Surface, Vendor};
use crate::domain::inference::value_objects::attempt_diagnostics::TerminalEvent;
use serde_json::Value;

#[path = "attempt_error_shape.rs"]
mod error_shape;
pub(super) fn failure_shape(
    data: &str,
    vendor: Vendor,
) -> Result<Option<TerminalEvent>, serde_json::Error> {
    error_shape::failure(data, vendor)
}

pub(super) struct Decoded {
    pub value: Option<Value>,
    pub terminal: Option<TerminalEvent>,
}
impl Decoded {
    pub fn completed(&self) -> bool {
        self.terminal == Some(TerminalEvent::ResponseCompleted)
    }
    pub fn failed(&self) -> bool {
        failed(self.terminal)
    }
}
pub(super) fn failed(terminal: Option<TerminalEvent>) -> bool {
    matches!(
        terminal,
        Some(
            TerminalEvent::Error
                | TerminalEvent::ResponseFailed
                | TerminalEvent::ResponseIncomplete
        )
    )
}
/// Whole-body Codex has always trimmed line ends, including Unicode whitespace.
/// Incremental dispatch keeps JSON whitespace acceptance unchanged.
pub(super) fn data(raw: &str, surface: Surface) -> &str {
    match surface {
        Surface::Chat | Surface::Assembled => raw.trim_end(),
        Surface::Incremental => raw,
    }
}
pub(super) fn terminal(value: &Value) -> Option<TerminalEvent> {
    match value["type"].as_str() {
        Some("error") => Some(TerminalEvent::Error),
        Some("response.failed") => Some(TerminalEvent::ResponseFailed),
        Some("response.incomplete") => Some(TerminalEvent::ResponseIncomplete),
        Some("response.completed") => Some(TerminalEvent::ResponseCompleted),
        None if super::attempt_profile::is_untyped_error_chunk(value) => Some(TerminalEvent::Error),
        _ => None,
    }
}
pub(super) fn decode(raw: &str, surface: Surface) -> Result<Decoded, serde_json::Error> {
    let data = data(raw, surface);
    if data.len() > super::provider_diagnostics::BODY_LIMIT {
        if let Some(terminal) = failure_shape(data, Vendor::Codex)? {
            // The strict scanner is also the actual consumer decision. No
            // oversized error Value or typed semantic declarations are built.
            return Ok(Decoded {
                value: None,
                terminal: Some(terminal),
            });
        }
    }
    let value: Value = serde_json::from_str(data)?;
    let terminal = terminal(&value);
    Ok(Decoded {
        value: Some(value),
        terminal,
    })
}

#[cfg(test)]
#[path = "codex_event_tests.rs"]
mod tests;
