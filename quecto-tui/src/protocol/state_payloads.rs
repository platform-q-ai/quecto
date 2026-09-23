//! Typed protocol values for TUI agent-state wire payloads (`get_state`,
//! `set_effort` / `set_model` success echoes, resume ack).
//!
//! Follows the mapper convention in [`crate::protocol::model_payloads`].

#[derive(serde::Deserialize)]
struct AdmissionWarningEntry {
    code: String,
    slot: String,
    message: String,
}

/// An unbound but usable provider slot: requests are not broker-gated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionBindingWarning {
    pub slot: String,
    pub message: String,
}

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
    /// True only for full snapshots that may authoritatively clear omitted fields.
    pub authoritative: bool,
    pub footer: GetStateFooterFields,
    /// Provider effort vocabulary (empty when absent or empty after sanitize).
    /// The active model's effort vocabulary as the agent reported it:
    /// `None` when the payload carried no `effortLevels` (not known yet),
    /// `Some(vec![])` when the model offers no effort control (#1996).
    pub effort_levels: Option<Vec<String>>,
    /// Raw `sessionKey` string when present (unsliced; caller extracts the name).
    pub session_key: Option<String>,
    /// Nested `workflow` object when present (still a Value so workflow mappers
    /// own its interpretation).
    pub workflow: Option<serde_json::Value>,
    /// Bounded inference-admission view when the agent shares an authority
    /// (#1679 P4); absent otherwise.
    pub admission: Option<crate::protocol::admission_payloads::AdmissionView>,
    /// Sanitized, typed startup warnings from the agent socket.
    pub admission_warnings: Vec<AdmissionBindingWarning>,
    /// A full warning list was supplied without malformed-only content.
    pub admission_warnings_authoritative: bool,
}

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawFooterFields {
    #[serde(default)]
    model: serde_json::Value,
    #[serde(default)]
    max_context_tokens: serde_json::Value,
    #[serde(default)]
    effort: serde_json::Value,
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
    let raw: RawFooterFields = serde_json::from_value(data.clone()).unwrap_or_default();
    let model = raw.model.as_str().map(sanitize);
    let max_context_tokens = raw.max_context_tokens.as_u64();
    let effort = raw.effort.as_str().map(sanitize);
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
        });
    let session_key = data
        .get("sessionKey")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let workflow = data.get("workflow").cloned();
    // `Null` for a missing key: the mapper answers `None` for anything but an
    // object, so no raw key lookup is needed here.
    let admission =
        crate::protocol::admission_payloads::parse_admission(&data["admission"], sanitize);
    let warning_values = data
        .get("admissionWarnings")
        .and_then(serde_json::Value::as_array);
    let mut warning_list_valid = true;
    let admission_warnings = warning_values
        .into_iter()
        .flatten()
        .take(64)
        .filter_map(|value| {
            let Ok(warning) = serde_json::from_value::<AdmissionWarningEntry>(value.clone()) else {
                warning_list_valid = false;
                return None;
            };
            if warning.code == "admission_binding_missing" {
                let slot = sanitize(&warning.slot);
                let message = sanitize(&warning.message);
                if !slot.is_empty() && !message.is_empty() {
                    return Some(AdmissionBindingWarning { slot, message });
                }
            }
            warning_list_valid = false;
            None
        })
        .collect::<Vec<_>>();
    let admission_warnings_authoritative =
        matches!(warning_values, Some(values) if values.len() <= 64) && warning_list_valid;
    GetStateSnapshot {
        authoritative: crate::protocol::presentation_payloads::bool_field(data, "unchanged")
            != Some(true),
        footer,
        effort_levels,
        session_key,
        workflow,
        admission,
        admission_warnings,
        admission_warnings_authoritative,
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
pub fn parse_set_model_id(
    data: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> Option<String> {
    data.get("model").and_then(|v| v.as_str()).map(sanitize)
}

/// What a successful `resume_session` answer tells the client (#1726).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSessionAck {
    /// Display name of the resumed session.
    pub name: String,
    /// Durable session key (`cli:<name>`) when the agent reported one.
    pub session_key: Option<String>,
}

#[cfg(test)]
#[path = "state_payloads_tests.rs"]
mod tests;
