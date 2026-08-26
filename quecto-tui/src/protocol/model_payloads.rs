//! Typed protocol values for TUI model-registry wire payloads.
//!
//! # Mapper convention (#1220)
//!
//! This module is an instance of the TUI protocol-boundary mapper convention.
//! Every mapper in `protocol/` obeys the same four rules:
//!
//! 1. **Input is raw wire JSON, output is a typed protocol value.** A mapper
//!    takes `&serde_json::Value` (the shape the infrastructure client receives
//!    from UDS) and returns a plain data type owned by the protocol layer.
//!    Feature controllers and views consume the typed value and never re-read
//!    the JSON.
//! 2. **Total, never failing on shape.** Malformed, legacy, and unknown payloads
//!    map to an empty/defaulted result rather than a panic or an error the UI
//!    must handle, unless the distinction is itself user-visible (see
//!    `session_payloads::ResumeMessagesError`).
//! 3. **The protocol layer owns no presentation types.** Mappers must not name
//!    presentation-layer types, so the returned value is a neutral DTO that the
//!    feature/view owner converts into its own view model at the seam.
//! 4. **Parity quirks live here, documented.** Legacy field fallbacks and
//!    sanitization rules preserved for zero-behaviour-change parity belong
//!    inside the mapper next to the canonical rules, never re-implemented at a
//!    consuming call site.

/// A model advertised by the harness in a `list_models` response.
///
/// Field derivation is the canonical, parity-preserving contract:
/// - `id`: `model` wins over the legacy `id` field; control characters are
///   stripped; entries whose id is absent, non-string, or empty after
///   sanitization are dropped entirely.
/// - `provider`: an explicit `provider` field wins; otherwise the slash prefix
///   of the id is inferred; otherwise the literal label `Model`.
/// - `auth`: sanitized, and dropped when empty so the view renders no label
///   rather than an empty one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelListEntry {
    pub id: String,
    pub provider: String,
    pub auth: Option<String>,
}

/// Map a `list_models` response payload into typed model entries.
///
/// A missing `models` key, a non-array `models` value, and an empty array all
/// map to no entries. Order is preserved. Single pass, one output `Vec`.
///
/// `sanitize` is injected by the caller because control-character stripping is
/// a presentation concern owned by the feature/view layer, which the protocol
/// layer must not depend on (rule 3). All *derivation* rules — including the
/// empty-after-sanitization skip — stay here (rule 4).
pub fn parse_model_list(
    data: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> Vec<ModelListEntry> {
    let Some(models) = data.get("models").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    models
        .iter()
        .filter_map(|model| parse_model_list_entry(model, sanitize))
        .collect()
}

fn parse_model_list_entry(
    model: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> Option<ModelListEntry> {
    // Legacy parity: older harness payloads carried the identifier as `id`,
    // current ones as `model`. `model` wins when both are present.
    let raw_model = model
        .get("model")
        .or_else(|| model.get("id"))
        .and_then(|v| v.as_str())?;
    let id = sanitize(raw_model);
    if id.is_empty() {
        return None;
    }
    let provider = sanitize(
        model
            .get("provider")
            .and_then(|v| v.as_str())
            .or_else(|| id.split_once('/').map(|(provider, _)| provider))
            .unwrap_or("Model"),
    );
    let auth = model
        .get("auth")
        .and_then(|v| v.as_str())
        .map(sanitize)
        .filter(|s| !s.is_empty());
    Some(ModelListEntry { id, provider, auth })
}

/// The rendered view of a `refresh_models` response: one human-readable
/// summary per source outcome, plus whether any outcome was unsuccessful.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RefreshOutcomeLines {
    pub summaries: Vec<String>,
    pub any_unsuccessful: bool,
}

/// Map a `refresh_models` payload into per-source summary lines. Unknown or
/// malformed outcome entries are skipped; all rendered text is sanitized at
/// this protocol boundary like the model-list mapper (#1220).
pub fn parse_refresh_outcomes(
    data: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> RefreshOutcomeLines {
    let mut lines = RefreshOutcomeLines::default();
    let Some(outcomes) = data.get("outcomes").and_then(|v| v.as_array()) else {
        return lines;
    };
    for outcome in outcomes {
        let Some(source) = outcome.get("source").and_then(|v| v.as_str()) else {
            continue;
        };
        let source = sanitize(source);
        let status = outcome.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let reason = outcome
            .get("reason")
            .and_then(|v| v.as_str())
            .map(sanitize)
            .filter(|s| !s.is_empty());
        let summary = match status {
            "updated" => {
                let models = outcome.get("models").and_then(|v| v.as_u64()).unwrap_or(0);
                format!("{source}: {models} model(s)")
            }
            "unchanged" => format!("{source}: unchanged"),
            "unsupported" => {
                lines.any_unsuccessful = true;
                match &reason {
                    Some(reason) => format!("{source}: not refreshable ({reason})"),
                    None => format!("{source}: not refreshable"),
                }
            }
            "failed" => {
                lines.any_unsuccessful = true;
                match &reason {
                    Some(reason) => format!("{source}: failed ({reason})"),
                    None => format!("{source}: failed"),
                }
            }
            "cancelled" => {
                lines.any_unsuccessful = true;
                format!("{source}: cancelled")
            }
            _ => continue,
        };
        lines.summaries.push(summary);
    }
    lines
}

#[cfg(test)]
#[path = "model_payloads_tests.rs"]
mod tests;
