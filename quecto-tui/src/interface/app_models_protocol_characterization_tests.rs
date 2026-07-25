//! Characterization of the `list_models` protocol surface (#1220).
//!
//! These tests pin the OBSERVABLE outcome of delivering a real `list_models`
//! response payload to the app: what the model-selector overlay renders, and
//! whether the overlay opens. They are written against the unmodified code so
//! that moving payload interpretation to a protocol mapper is provably
//! behaviour-preserving.

use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

/// Render the model-selector overlay as plain text (ANSI stripped).
fn overlay_text(app: &mut App) -> String {
    app.compose_frame()
        .iter()
        .map(|l| super::app_methods::strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Open the selector by delivering `payload` as the `list_models` response and
/// return the rendered overlay text.
async fn rendered_for(payload: serde_json::Value) -> String {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(payload));
    assert!(
        a.model_selector.is_some(),
        "a delivered list_models response must open the pending selector"
    );
    overlay_text(a)
}

// ── entry derivation ────────────────────────────────────────────────

#[tokio::test]
async fn renders_model_field_entries_in_payload_order() {
    let text = rendered_for(serde_json::json!({
        "models": [
            { "model": "anthropic/claude-3-opus" },
            { "model": "openai/gpt-4" }
        ]
    }))
    .await;

    let first = text
        .find("claude-3-opus")
        .expect("first model must be rendered");
    let second = text.find("gpt-4").expect("second model must be rendered");
    assert!(
        first < second,
        "models must render in payload order, got:\n{text}"
    );
}

#[tokio::test]
async fn falls_back_to_id_when_model_field_absent() {
    let text = rendered_for(serde_json::json!({
        "models": [ { "id": "mistral/mistral-large" } ]
    }))
    .await;
    assert!(
        text.contains("mistral-large"),
        "an entry with only `id` must still render, got:\n{text}"
    );
}

#[tokio::test]
async fn model_field_wins_over_id_field() {
    let text = rendered_for(serde_json::json!({
        "models": [ { "model": "from/model-field", "id": "from/id-field" } ]
    }))
    .await;
    assert!(
        text.contains("model-field"),
        "`model` must take precedence over `id`, got:\n{text}"
    );
    assert!(
        !text.contains("id-field"),
        "`id` must be ignored when `model` is present, got:\n{text}"
    );
}

// ── provider derivation ─────────────────────────────────────────────

#[tokio::test]
async fn provider_is_inferred_from_slash_prefix() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [ { "model": "anthropic/claude-3-opus" } ]
    })));
    // Asserted on the parsed provider, not the frame: the id is rendered as the
    // row label, so a frame match on "anthropic" would survive deleting the
    // slash-inference branch entirely.
    assert_eq!(
        a.model_registry.entries[0].provider, "anthropic",
        "the slash prefix must be inferred as the provider"
    );
    assert_eq!(
        a.model_registry.entries[0].id, "anthropic/claude-3-opus",
        "the full id must be preserved when the provider is inferred"
    );
    assert!(
        !a.model_registry.entries[0].is_current,
        "a freshly parsed entry must never be marked current"
    );
}

#[tokio::test]
async fn explicit_provider_overrides_slash_inference() {
    let text = rendered_for(serde_json::json!({
        "models": [ { "model": "custom/model", "provider": "my-provider" } ]
    }))
    .await;
    assert!(
        text.contains("my-provider"),
        "an explicit provider must be shown, got:\n{text}"
    );
}

#[tokio::test]
async fn provider_defaults_to_model_label_without_slash() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [ { "model": "local-model" } ]
    })));
    assert_eq!(
        a.model_registry.entries[0].provider, "Model",
        "a slashless id with no explicit provider must fall back to the `Model` label"
    );
    let text = overlay_text(a);
    assert!(
        text.contains("local-model"),
        "the entry itself must still render, got:\n{text}"
    );
}

// ── auth rendering ──────────────────────────────────────────────────

#[tokio::test]
async fn auth_label_renders_when_present() {
    let text = rendered_for(serde_json::json!({
        "models": [ { "model": "openai/gpt-4", "auth": "api-key" } ]
    }))
    .await;
    assert!(
        text.contains("api-key"),
        "a non-empty auth value must render, got:\n{text}"
    );
}

#[tokio::test]
async fn empty_auth_string_yields_no_auth_value() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [
            { "model": "openai/gpt-4", "auth": "" },
            { "model": "openai/gpt-5", "auth": "api-key" }
        ]
    })));
    assert_eq!(
        a.model_registry.entries[0].auth, None,
        "an empty auth string must be dropped, not carried as an empty label"
    );
    assert_eq!(
        a.model_registry.entries[1].auth.as_deref(),
        Some("api-key"),
        "a non-empty auth value must be preserved"
    );
}

// ── control-character sanitization ──────────────────────────────────

#[tokio::test]
async fn control_characters_are_stripped_from_rendered_fields() {
    let text = rendered_for(serde_json::json!({
        "models": [ {
            "model": "pro\u{0007}vider/mo\u{0000}del",
            "provider": "pro\u{0007}vider",
            "auth": "api\u{0007}-key"
        } ]
    }))
    .await;
    assert!(
        text.contains("provider/model") && text.contains("api-key"),
        "sanitized values must render, got:\n{text}"
    );
    assert!(
        !text.contains('\u{0007}') && !text.contains('\u{0000}'),
        "control characters must never reach the rendered frame"
    );
}

// ── skipped entries ─────────────────────────────────────────────────

#[tokio::test]
async fn empty_model_id_entry_is_skipped_but_siblings_render() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [ { "model": "" }, { "model": "valid/model" } ]
    })));
    assert_eq!(
        a.model_registry.entries.len(),
        1,
        "an entry with an empty id must be skipped"
    );
    let text = overlay_text(a);
    assert!(
        text.contains("valid/model"),
        "the valid sibling must still render, got:\n{text}"
    );
    assert_eq!(
        a.model_registry.entries[0].id, "valid/model",
        "the surviving entry must be the valid sibling, not a coerced placeholder"
    );
}

#[tokio::test]
async fn non_string_model_id_entry_is_skipped() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [ { "model": 123 }, { "model": "valid/model" } ]
    })));
    assert_eq!(
        a.model_registry.entries.len(),
        1,
        "a non-string model id must be skipped"
    );
    assert_eq!(
        a.model_registry.entries[0].id, "valid/model",
        "the surviving entry must be the valid sibling"
    );
}

// ── malformed / legacy payload boundaries ───────────────────────────

#[tokio::test]
async fn empty_models_array_opens_selector_with_no_entries() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({ "models": [] })));
    assert!(
        a.model_selector.is_some(),
        "an empty list must still open the selector"
    );
    assert!(a.model_registry.entries.is_empty());
}

#[tokio::test]
async fn missing_models_key_yields_no_entries() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({})));
    assert!(a.model_selector.is_some());
    assert!(
        a.model_registry.entries.is_empty(),
        "a payload without `models` must yield no entries"
    );
}

#[tokio::test]
async fn models_not_an_array_yields_no_entries() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({ "models": "not an array" })));
    assert!(a.model_selector.is_some());
    assert!(
        a.model_registry.entries.is_empty(),
        "a non-array `models` field must yield no entries"
    );
}

// ── pending-open lifecycle ──────────────────────────────────────────

#[tokio::test]
async fn absent_payload_opens_selector_and_keeps_cached_entries() {
    let mut h = harness().await;
    let a = h.app_mut();
    // Prime the cache with a successful list first.
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [ { "model": "cached/entry" } ]
    })));
    a.model_selector = None;

    // A later open answered with no payload must fall back to the cache.
    a.open_model_selector();
    a.handle_list_models(None);
    assert!(
        a.model_selector.is_some(),
        "a payload-less response must still open the selector"
    );
    assert_eq!(
        a.model_registry.entries.len(),
        1,
        "a payload-less response must not clear cached entries"
    );
    let text = overlay_text(a);
    assert!(
        text.contains("cached/entry"),
        "cached entries must render, got:\n{text}"
    );
}

#[tokio::test]
async fn delivery_without_pending_open_updates_cache_without_opening() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_list_models(Some(serde_json::json!({
        "models": [ { "model": "quiet/entry" } ]
    })));
    assert!(
        a.model_selector.is_none(),
        "an unsolicited list must not open the selector"
    );
    assert_eq!(
        a.model_registry.entries.len(),
        1,
        "an unsolicited list must still refresh the cache"
    );
}

#[tokio::test]
async fn pending_open_flag_clears_exactly_once() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    assert!(
        a.model_registry.open_pending,
        "opening defers until the fresh list arrives"
    );
    a.handle_list_models(Some(serde_json::json!({ "models": [] })));
    assert!(
        !a.model_registry.open_pending,
        "the pending flag must clear when the list arrives"
    );
    a.model_selector = None;
    a.handle_list_models(Some(serde_json::json!({ "models": [] })));
    assert!(
        a.model_selector.is_none(),
        "a later unsolicited list must not re-open the selector"
    );
}

// ── command emission and ordering ───────────────────────────────────

/// Opening the selector must emit exactly one `list_models` command. Pinned on
/// the wire, not on the pending flag, so a mapper refactor cannot silently add,
/// drop, or reorder the request.
#[tokio::test]
async fn open_model_selector_emits_exactly_one_list_models() {
    let mut h = harness().await;
    h.app_mut().open_model_selector();
    let cmds = h.drain_commands().await;
    let listed: Vec<&String> = cmds.iter().filter(|c| c.contains("list_models")).collect();
    assert_eq!(
        listed.len(),
        1,
        "opening the selector must emit exactly one list_models, got:\n{cmds:?}"
    );
}

/// A second open while the first request is still pending must NOT re-request:
/// the deferred-open guard is the only thing preventing duplicate traffic.
#[tokio::test]
async fn second_open_while_pending_emits_no_duplicate_list_models() {
    let mut h = harness().await;
    h.app_mut().open_model_selector();
    h.app_mut().open_model_selector();
    let cmds = h.drain_commands().await;
    let listed: Vec<&String> = cmds.iter().filter(|c| c.contains("list_models")).collect();
    assert_eq!(
        listed.len(),
        1,
        "a re-entrant open while pending must not duplicate list_models, got:\n{cmds:?}"
    );
}

/// Selecting an entry emits `set_model` on the wire with the chosen id.
#[tokio::test]
async fn selector_selection_emits_set_model_command() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [ { "model": "openai/gpt-4" } ]
    })));
    a.handle_model_selector_key(&Key::Enter);
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|c| c.contains("set_model") && c.contains("openai/gpt-4")),
        "selecting an entry must emit set_model for that id, got:\n{cmds:?}"
    );
}

/// Closes the composition gap flagged in review on #1235: the mapper fixtures
/// inject a deliberately weaker sanitizer stand-in, so ANSI escape sequences and
/// Trojan-Source bidi controls were pinned only at the `ansi.rs` unit seam and
/// never carried through the mapper into a rendered frame. This runs the real
/// `sanitize_control` end-to-end.
#[tokio::test]
async fn ansi_and_bidi_controls_are_stripped_through_the_mapper() {
    let text = rendered_for(serde_json::json!({
        "models": [ { "model": "prov\u{202E}ider/mo\u{1b}[31mdel" } ]
    }))
    .await;
    assert!(
        text.contains("provider/model"),
        "the ANSI sequence and bidi control must be stripped end-to-end, got:\n{text}"
    );
    assert!(
        !text.contains('\u{202E}') && !text.contains('\u{1b}') && !text.contains("[31m"),
        "no bidi control, ESC byte, or escape-sequence remnant may reach the frame, got:\n{text}"
    );
}
