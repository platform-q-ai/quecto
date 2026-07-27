use super::*;

// ── parse_model_entries: basic parsing ──────────────────────────

#[test]
fn parse_empty_models_array() {
    let data = serde_json::json!({ "models": [] });
    let entries = parse_model_entries(&data);
    assert!(entries.is_empty());
}

#[test]
fn parse_models_with_model_field() {
    let data = serde_json::json!({
        "models": [
            { "model": "anthropic/claude-3-opus" },
            { "model": "openai/gpt-4" }
        ]
    });
    let entries = parse_model_entries(&data);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].id, "anthropic/claude-3-opus");
    assert_eq!(entries[1].id, "openai/gpt-4");
}

#[test]
fn parse_models_with_id_field_fallback() {
    // Some providers return "id" instead of "model".
    let data = serde_json::json!({
        "models": [
            { "id": "mistral/mistral-large" }
        ]
    });
    let entries = parse_model_entries(&data);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "mistral/mistral-large");
}

// ── parse_model_entries: provider inference ─────────────────────

#[test]
fn parse_infers_provider_from_slash_prefix() {
    let data = serde_json::json!({
        "models": [
            { "model": "anthropic/claude-3-opus" }
        ]
    });
    let entries = parse_model_entries(&data);
    assert_eq!(entries[0].provider, "anthropic");
}

#[test]
fn parse_explicit_provider_overrides_slash_inference() {
    let data = serde_json::json!({
        "models": [
            { "model": "custom/model", "provider": "my-provider" }
        ]
    });
    let entries = parse_model_entries(&data);
    assert_eq!(entries[0].provider, "my-provider");
}

#[test]
fn parse_provider_defaults_to_model_when_no_slash() {
    let data = serde_json::json!({
        "models": [
            { "model": "local-model" }
        ]
    });
    let entries = parse_model_entries(&data);
    assert_eq!(entries[0].provider, "Model");
}

// ── parse_model_entries: auth field ─────────────────────────────

#[test]
fn parse_auth_present() {
    let data = serde_json::json!({
        "models": [
            { "model": "openai/gpt-4", "auth": "api-key" }
        ]
    });
    let entries = parse_model_entries(&data);
    assert_eq!(entries[0].auth.as_deref(), Some("api-key"));
}

#[test]
fn parse_auth_absent_is_none() {
    let data = serde_json::json!({
        "models": [
            { "model": "openai/gpt-4" }
        ]
    });
    let entries = parse_model_entries(&data);
    assert!(entries[0].auth.is_none());
}

#[test]
fn parse_auth_empty_string_is_filtered_out() {
    let data = serde_json::json!({
        "models": [
            { "model": "openai/gpt-4", "auth": "" }
        ]
    });
    let entries = parse_model_entries(&data);
    assert!(
        entries[0].auth.is_none(),
        "empty auth string should be filtered"
    );
}

// ── parse_model_entries: sanitization ───────────────────────────

#[test]
fn parse_strips_control_chars_from_model_id() {
    let data = serde_json::json!({
        "models": [
            { "model": "model\u{0007}with\u{0000}control" }
        ]
    });
    let entries = parse_model_entries(&data);
    assert_eq!(entries[0].id, "modelwithcontrol");
}

#[test]
fn parse_strips_control_chars_from_provider() {
    let data = serde_json::json!({
        "models": [
            { "model": "test/model", "provider": "pro\u{0007}vider" }
        ]
    });
    let entries = parse_model_entries(&data);
    assert_eq!(entries[0].provider, "provider");
}

#[test]
fn parse_strips_control_chars_from_auth() {
    let data = serde_json::json!({
        "models": [
            { "model": "test/model", "auth": "api\u{0007}-key" }
        ]
    });
    let entries = parse_model_entries(&data);
    assert_eq!(entries[0].auth.as_deref(), Some("api-key"));
}

// ── parse_model_entries: edge cases ─────────────────────────────

#[test]
fn parse_empty_model_id_is_skipped() {
    let data = serde_json::json!({
        "models": [
            { "model": "" },
            { "model": "valid/model" }
        ]
    });
    let entries = parse_model_entries(&data);
    assert_eq!(entries.len(), 1, "empty model id should be skipped");
    assert_eq!(entries[0].id, "valid/model");
}

#[test]
fn parse_missing_models_key_returns_empty() {
    let data = serde_json::json!({});
    let entries = parse_model_entries(&data);
    assert!(entries.is_empty());
}

#[test]
fn parse_models_not_array_returns_empty() {
    let data = serde_json::json!({ "models": "not an array" });
    let entries = parse_model_entries(&data);
    assert!(entries.is_empty());
}

#[test]
fn parse_model_without_string_model_field_skipped() {
    let data = serde_json::json!({
        "models": [
            { "model": 123 },
            { "model": "valid/model" }
        ]
    });
    let entries = parse_model_entries(&data);
    assert_eq!(entries.len(), 1);
}

#[test]
fn parse_is_current_starts_false() {
    let data = serde_json::json!({
        "models": [{ "model": "test/model" }]
    });
    let entries = parse_model_entries(&data);
    assert!(!entries[0].is_current);
}

#[test]
fn parse_multiple_mixed_entries() {
    let data = serde_json::json!({
        "models": [
            { "model": "anthropic/claude", "provider": "anthropic", "auth": "api-key" },
            { "id": "openai/gpt-4" },
            { "model": "local" },
            { "model": "" },
            { "model": "mistral/mistral", "auth": "" }
        ]
    });
    let entries = parse_model_entries(&data);
    assert_eq!(entries.len(), 4); // one empty-id entry skipped
    assert_eq!(entries[0].id, "anthropic/claude");
    assert_eq!(entries[1].id, "openai/gpt-4");
    assert_eq!(entries[2].id, "local");
    assert_eq!(entries[3].id, "mistral/mistral");
    // Auth: first has it, second/third don't, fourth was empty → filtered.
    assert!(entries[0].auth.is_some());
    assert!(entries[1].auth.is_none());
    assert!(entries[2].auth.is_none());
    assert!(entries[3].auth.is_none());
}
