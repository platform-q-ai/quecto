//! The guardrail audit: tool calls a permission rule refused (#2285).

use serde_json::Value;

/// A tool call a permission rule refused, from `result.permission_denials`.
#[derive(Debug, Clone, PartialEq)]
pub struct GuardrailDenial {
    pub tool_name: String,
    pub tool_use_id: String,
    pub tool_input: Value,
    /// The turn (0-based) whose result reported it.
    pub turn: usize,
}
