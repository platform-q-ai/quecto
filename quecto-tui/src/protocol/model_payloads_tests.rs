//! Mapper tests for the `list_models` protocol boundary (#1220).
//!
//! Covers valid, legacy, and malformed fixtures at the mapper seam. The
//! observable end-to-end behaviour is pinned separately by the interface
//! characterization suite.

use super::*;

/// Stand-in for the interface sanitizer: strips ASCII control characters.
///
/// DELIBERATELY WEAKER than the production `interface::ansi::sanitize_control`,
/// which is ANSI-segment aware (`"\u{1b}[31mred"` -> `"red"`, not `"[31mred"`)
/// and also strips Trojan-Source bidi controls (`"\u{202E}"` -> `""`). The
/// protocol layer may not name `interface::` types (rule 3), so these
/// fixtures pin the mapper's *contract* — "whatever the sanitizer returns, an
/// id that comes back empty is dropped rather than rendered blank" — and not
/// the sanitizer's semantics. Those are pinned at their own seam by
/// `interface/ansi_tests.rs`, and end-to-end through the real wiring by the
/// frozen characterization suite. Do not read a passing test here as evidence
/// about ANSI or bidi handling in production.
fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

fn parse(value: serde_json::Value) -> Vec<ModelListEntry> {
    parse_model_list(&value, &sanitize)
}

// ── valid payloads ──────────────────────────────────────────────────

#[test]
fn maps_models_in_payload_order() {
    let entries = parse(serde_json::json!({
        "models": [ { "model": "a/one" }, { "model": "b/two" } ]
    }));
    assert_eq!(
        entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        vec!["a/one", "b/two"],
        "entries must preserve payload order"
    );
}

#[test]
fn infers_provider_from_slash_prefix() {
    let entries = parse(serde_json::json!({
        "models": [ { "model": "anthropic/claude-3-opus" } ]
    }));
    assert_eq!(entries[0].provider, "anthropic");
}

#[test]
fn explicit_provider_wins_over_slash_prefix() {
    let entries = parse(serde_json::json!({
        "models": [ { "model": "custom/model", "provider": "my-provider" } ]
    }));
    assert_eq!(entries[0].provider, "my-provider");
}

#[test]
fn provider_falls_back_to_model_label_without_slash() {
    let entries = parse(serde_json::json!({ "models": [ { "model": "local-model" } ] }));
    assert_eq!(entries[0].provider, "Model");
}

#[test]
fn auth_is_preserved_when_non_empty() {
    let entries = parse(serde_json::json!({
        "models": [ { "model": "openai/gpt-4", "auth": "api-key" } ]
    }));
    assert_eq!(entries[0].auth.as_deref(), Some("api-key"));
}

#[test]
fn empty_auth_is_dropped() {
    let entries = parse(serde_json::json!({
        "models": [ { "model": "openai/gpt-4", "auth": "" } ]
    }));
    assert_eq!(
        entries[0].auth, None,
        "an empty auth must not become a label"
    );
}

#[test]
fn absent_auth_is_none() {
    let entries = parse(serde_json::json!({ "models": [ { "model": "openai/gpt-4" } ] }));
    assert_eq!(entries[0].auth, None);
}

// ── legacy payloads ─────────────────────────────────────────────────

#[test]
fn legacy_id_field_is_used_when_model_is_absent() {
    let entries = parse(serde_json::json!({ "models": [ { "id": "legacy/model" } ] }));
    assert_eq!(entries[0].id, "legacy/model");
}

#[test]
fn model_field_wins_over_legacy_id_field() {
    let entries = parse(serde_json::json!({
        "models": [ { "model": "new/model", "id": "legacy/model" } ]
    }));
    assert_eq!(entries[0].id, "new/model");
}

// ── malformed payloads ──────────────────────────────────────────────

#[test]
fn missing_models_key_maps_to_no_entries() {
    assert!(parse(serde_json::json!({})).is_empty());
}

#[test]
fn non_array_models_maps_to_no_entries() {
    assert!(parse(serde_json::json!({ "models": "not an array" })).is_empty());
}

#[test]
fn empty_models_array_maps_to_no_entries() {
    assert!(parse(serde_json::json!({ "models": [] })).is_empty());
}

#[test]
fn entry_without_any_identifier_is_dropped() {
    let entries = parse(serde_json::json!({
        "models": [ { "provider": "orphan" }, { "model": "kept/model" } ]
    }));
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "kept/model");
}

#[test]
fn non_string_identifier_is_dropped() {
    let entries = parse(serde_json::json!({
        "models": [ { "model": 123 }, { "model": "kept/model" } ]
    }));
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "kept/model");
}

#[test]
fn identifier_empty_after_sanitization_is_dropped() {
    let entries = parse(serde_json::json!({
        "models": [ { "model": "\u{0007}" }, { "model": "kept/model" } ]
    }));
    assert_eq!(
        entries.len(),
        1,
        "an id that sanitizes away entirely must be dropped, not rendered blank"
    );
    assert_eq!(entries[0].id, "kept/model");
}

#[test]
fn control_characters_are_stripped_from_every_field() {
    let entries = parse(serde_json::json!({
        "models": [ {
            "model": "pro\u{0007}vider/mo\u{0000}del",
            "provider": "pro\u{0007}vider",
            "auth": "api\u{0007}-key"
        } ]
    }));
    assert_eq!(entries[0].id, "provider/model");
    assert_eq!(entries[0].provider, "provider");
    assert_eq!(entries[0].auth.as_deref(), Some("api-key"));
}

#[test]
fn non_object_entries_are_dropped() {
    let entries = parse(serde_json::json!({
        "models": [ "just-a-string", 7, null, { "model": "kept/model" } ]
    }));
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "kept/model");
}

#[test]
fn refresh_failures_and_catalogue_errors_reach_the_view() {
    let sanitize = |s: &str| s.to_string();

    let partial = serde_json::json!({"sources": [
        {"source": "openai-api", "status": "refreshed", "models": 12},
        {"source": "spark", "status": "failed", "reason": "connection refused"},
        {"source": "anthropic-oauth", "status": "skipped", "reason": "oauth"},
    ]});
    assert_eq!(
        parse_refresh_failures(&partial, &sanitize),
        ["spark: connection refused"],
        "a refresh that partly failed must name what failed"
    );
    assert!(parse_refresh_failures(&serde_json::json!({}), &sanitize).is_empty());

    assert_eq!(
        parse_model_list_error(
            &serde_json::json!({"models": [], "error": "failed to parse models.json"}),
            &sanitize
        )
        .as_deref(),
        Some("failed to parse models.json")
    );
    assert_eq!(
        parse_model_list_error(&serde_json::json!({"models": []}), &sanitize),
        None
    );
}
