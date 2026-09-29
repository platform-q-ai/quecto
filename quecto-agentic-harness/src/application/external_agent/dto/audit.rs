//! The guardrail audit: tool calls a permission rule refused (#2285).

use serde_json::Value;

/// The projection keeps this many of the latest denials; older ones are
/// counted in [`super::SessionTotals::guardrail_denials`] only.
pub const GUARDRAIL_AUDIT_CAPACITY: usize = 32;

/// The projection keeps this many of the latest admission warnings;
/// older ones are counted in [`super::SessionTotals::admission_warnings`].
pub const ADMISSION_WARNING_CAPACITY: usize = 16;

/// A denial's tool input is kept as a JSON preview of at most this many
/// bytes.
pub const AUDIT_INPUT_PREVIEW_BYTES: usize = 512;

/// A tool call a permission rule refused, from `result.permission_denials`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailDenial {
    pub tool_name: String,
    pub tool_use_id: String,
    /// The tool input as JSON, cut to [`AUDIT_INPUT_PREVIEW_BYTES`] on a
    /// character boundary.
    pub tool_input_preview: String,
    /// The turn (0-based) whose result reported it.
    pub turn: usize,
}

/// `input` as JSON, cut to at most [`AUDIT_INPUT_PREVIEW_BYTES`] on a
/// character boundary.
pub fn input_preview(input: &Value) -> String {
    let mut json = input.to_string();
    let mut end = json.len().min(AUDIT_INPUT_PREVIEW_BYTES);
    while !json.is_char_boundary(end) {
        let before = end;
        end -= 1;
        assert!(end < before, "backing off to a char boundary moves back");
    }
    json.truncate(end);
    json
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
