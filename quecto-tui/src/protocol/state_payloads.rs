//! Typed protocol values for TUI agent-state wire payloads (`get_state`,
//! `set_effort` / `set_model` success echoes, resume ack).
//!
//! Follows the mapper convention in [`crate::protocol::model_payloads`].

/// Footer-relevant fields from a successful `get_state` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetStateFooterFields {
    /// Sanitized model id when present.
    pub model: Option<String>,
    /// Context window size when present.
    pub max_context_tokens: Option<u64>,
    /// Effort level: `None` means explicit null or missing key (default).
    pub effort: Option<String>,
}

/// Full `get_state` fields the App response path consumes beyond the footer.
#[derive(Debug, Clone, PartialEq)]
pub struct GetStateSnapshot {
    pub footer: GetStateFooterFields,
    /// Provider effort vocabulary (empty when absent or empty after sanitize).
    pub effort_levels: Vec<String>,
    /// Raw `sessionKey` string when present (unsliced; caller extracts the name).
    pub session_key: Option<String>,
    /// Nested `workflow` object when present (still a Value so workflow mappers
    /// own its interpretation).
    pub workflow: Option<serde_json::Value>,
}

/// Parse footer fields from a `get_state` payload.
///
/// Parity: missing `effort` and explicit `"effort": null` both yield
/// `effort: None` so the footer always reflects the effective default.
pub fn parse_get_state_footer(
    data: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> GetStateFooterFields {
    // Parity: do not drop empty-after-sanitize strings — historical footer
    // behaviour applied whatever sanitize returned, including empty.
    let model = data.get("model").and_then(|m| m.as_str()).map(sanitize);
    let max_context_tokens = data.get("maxContextTokens").and_then(|v| v.as_u64());
    let effort = data.get("effort").and_then(|v| v.as_str()).map(sanitize);
    GetStateFooterFields {
        model,
        max_context_tokens,
        effort,
    }
}

/// Parse the full `get_state` snapshot used by the App response path.
pub fn parse_get_state(
    data: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> GetStateSnapshot {
    let footer = parse_get_state_footer(data, sanitize);
    let effort_levels = data
        .get("effortLevels")
        .and_then(|v| v.as_array())
        .map(|levels| {
            levels
                .iter()
                .filter_map(|l| l.as_str())
                .map(sanitize)
                .collect()
        })
        .unwrap_or_default();
    let session_key = data
        .get("sessionKey")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let workflow = data.get("workflow").cloned();
    GetStateSnapshot {
        footer,
        effort_levels,
        session_key,
        workflow,
    }
}

/// Extract the effort level echoed on a successful `set_effort` response.
pub fn parse_set_effort_level(
    data: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> Option<String> {
    data.get("effort").and_then(|v| v.as_str()).map(sanitize)
}

/// Extract the model id echoed on a successful `set_model` response.
/// A successful `set_model` response: the echoed model, and why it cannot
/// currently run when the agent recorded a switch it knows is unusable.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct SetModelResponse {
    model: Option<String>,
    unavailable: Option<String>,
}

/// Both fields of a `set_model` response, mapped once.
pub fn parse_set_model(
    data: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> (Option<String>, Option<String>) {
    let parsed: SetModelResponse = serde_json::from_value(data.clone()).unwrap_or_default();
    (
        parsed.model.as_deref().map(sanitize),
        parsed.unavailable.as_deref().map(sanitize),
    )
}

pub fn parse_set_model_id(
    data: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> Option<String> {
    parse_set_model(data, sanitize).0
}

/// Extract the session name from a successful `resume_session` response.
///
/// Missing or non-string `session` falls back to the literal `"session"` so the
/// toast stays user-visible (historical TUI behaviour).
pub fn parse_resume_session_name(data: &serde_json::Value) -> String {
    data.get("session")
        .and_then(|v| v.as_str())
        .unwrap_or("session")
        .to_string()
}

#[cfg(test)]
#[path = "state_payloads_tests.rs"]
mod tests;
