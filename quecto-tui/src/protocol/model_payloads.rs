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

use serde::Deserialize;

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
/// One source's outcome in a catalogue refresh response.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct RefreshOutcome {
    source: Option<String>,
    status: Option<String>,
    reason: Option<String>,
}

/// A catalogue refresh response: per-source outcomes.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct RefreshResponse {
    sources: Vec<RefreshOutcome>,
}

/// The sources a catalogue refresh could not refresh, as `source: reason`.
pub fn parse_refresh_failures(
    data: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> Vec<String> {
    let parsed = RefreshResponse::deserialize(data).unwrap_or_default();
    parsed
        .sources
        .into_iter()
        .filter(|outcome| outcome.status.as_deref() == Some("failed"))
        .map(|outcome| {
            sanitize(&format!(
                "{}: {}",
                outcome.source.as_deref().unwrap_or("unknown source"),
                outcome.reason.as_deref().unwrap_or("no reason reported"),
            ))
        })
        .collect()
}

/// A `list_models` response: the published list, and the catalogue error when
/// the file on disk could not be resolved into a new generation.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct ModelListResponse {
    /// Tolerant like the rows: version skew in one field must not empty the
    /// selector, which (with no fallback catalogue) would leave the user with
    /// no models and no explanation.
    #[serde(deserialize_with = "array_or_empty")]
    models: Vec<serde_json::Value>,
    #[serde(deserialize_with = "string_or_none")]
    error: Option<String>,
}

/// Read an array field, treating any other JSON type as absent.
fn array_or_empty<'de, D>(deserializer: D) -> Result<Vec<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Array(values) => values,
        _ => Vec::new(),
    })
}

/// A `list_models` response mapped once: the models to show, and the catalogue
/// error when the file on disk could not be resolved into a new generation.
pub fn parse_model_list_response(
    data: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> (Vec<ModelListEntry>, Option<String>) {
    // Borrowed, not cloned: a discovered catalogue can hold thousands of rows
    // and the selector re-requests this list on every open.
    let parsed = ModelListResponse::deserialize(data).unwrap_or_default();
    let entries = parsed
        .models
        .iter()
        .filter_map(|model| parse_model_list_entry(model, sanitize))
        .collect();
    (entries, parsed.error.as_deref().map(sanitize))
}

/// The catalogue error reported alongside a model list: the published list is
/// the last valid one, and the user needs to know the file on disk is broken.
pub fn parse_model_list_error(
    data: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> Option<String> {
    parse_model_list_response(data, sanitize).1
}

pub fn parse_model_list(
    data: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> Vec<ModelListEntry> {
    parse_model_list_response(data, sanitize).0
}

/// One listed model as the harness projects it.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct ModelListRow {
    /// Legacy parity: older harness payloads carried the identifier as `id`,
    /// current ones as `model`. `model` wins when both are present. Every field
    /// tolerates a wrong-typed value: one bad field must not drop a row whose
    /// identifier is perfectly good.
    #[serde(deserialize_with = "string_or_none")]
    model: Option<String>,
    #[serde(deserialize_with = "string_or_none")]
    id: Option<String>,
    #[serde(deserialize_with = "string_or_none")]
    provider: Option<String>,
    #[serde(deserialize_with = "string_or_none")]
    auth: Option<String>,
}

/// Read a string field, treating any other JSON type as absent.
fn string_or_none<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(value) => Some(value),
        _ => None,
    })
}

fn parse_model_list_entry(
    model: &serde_json::Value,
    sanitize: &dyn Fn(&str) -> String,
) -> Option<ModelListEntry> {
    let row = ModelListRow::deserialize(model).unwrap_or_default();
    let id = sanitize(row.model.as_deref().or(row.id.as_deref())?);
    if id.is_empty() {
        return None;
    }
    let provider = sanitize(
        row.provider
            .as_deref()
            .or_else(|| id.split_once('/').map(|(provider, _)| provider))
            .unwrap_or("Model"),
    );
    let auth = row
        .auth
        .as_deref()
        .map(sanitize)
        .filter(|auth| !auth.is_empty());
    Some(ModelListEntry { id, provider, auth })
}

#[cfg(test)]
#[path = "model_payloads_tests.rs"]
mod tests;
