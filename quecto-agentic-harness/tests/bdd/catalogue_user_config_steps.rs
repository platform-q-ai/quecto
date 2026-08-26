//! Steps for catalogue_user_config.feature (issue #1575, epic #1193 slice 5).
//!
//! Exercises the real user extension surface end-to-end over a temp base
//! directory: model add, stable-ID override, provider add to runnable,
//! unsupported transports, hot reload (pull-based re-resolve per ADR-0002),
//! secret-reference enforcement, and legacy `models.json` compatibility.

use std::sync::Arc;

use super::*;
use quecto::application::catalogue::ResolvedCatalogue;
use quecto::domain::catalogue::{CatalogueSnapshot, ModelRef, UnavailableReason};
use quecto::interface::cli::catalogue_bridge::resolve_and_publish_for;
use quecto::interface::cli::uds_models::list_models_data;

#[derive(Debug, Default)]
pub struct CatalogueUserConfigState {
    base_dir: Option<tempfile::TempDir>,
    /// The provider block a composable "adding provider" Given is building.
    provider_under_build: Option<String>,
    resolved: Option<ResolvedCatalogue>,
    /// The snapshot published before the file was rewritten, kept so hot
    /// reload can be told apart from a mere republish.
    snapshot_before_rewrite: Option<Arc<CatalogueSnapshot>>,
    uds_listing: Option<serde_json::Value>,
}

fn ucfg_base(world: &mut QuectoWorld) -> std::path::PathBuf {
    world
        .catalogue_user_config
        .base_dir
        .get_or_insert_with(|| tempfile::tempdir().expect("tempdir"))
        .path()
        .to_path_buf()
}

fn ucfg_write(world: &mut QuectoWorld, content: &serde_json::Value) {
    let base = ucfg_base(world);
    std::fs::write(base.join("models.json"), content.to_string()).expect("write models.json");
}

fn ucfg_read(world: &mut QuectoWorld) -> serde_json::Value {
    let base = ucfg_base(world);
    let raw = std::fs::read_to_string(base.join("models.json")).expect("read models.json");
    serde_json::from_str(&raw).expect("models.json is JSON")
}

fn ucfg_resolved(world: &QuectoWorld) -> &ResolvedCatalogue {
    world
        .catalogue_user_config
        .resolved
        .as_ref()
        .expect("catalogue resolved")
}

fn ucfg_find<'a>(
    resolved: &'a ResolvedCatalogue,
    qualified: &str,
) -> &'a quecto::domain::catalogue::CatalogueEntry {
    resolved
        .snapshot
        .find(&ModelRef::parse_qualified(qualified).expect("qualified id"))
        .unwrap_or_else(|| panic!("model '{qualified}' not in published snapshot"))
}

fn add_model_json(provider: &str, id: &str, name: &str) -> serde_json::Value {
    serde_json::json!({"providers": {provider: {
        "api": "openai-completions",
        "models": [{"id": id, "name": name}]
    }}})
}

#[given(expr = "a user catalogue file adding model {string} named {string} to provider {string}")]
fn ucfg_given_model_add(world: &mut QuectoWorld, id: String, name: String, provider: String) {
    ucfg_write(world, &add_model_json(&provider, &id, &name));
}

#[given(
    expr = "a user catalogue file overriding model {string} with name {string} and context window {int}"
)]
fn ucfg_given_override(world: &mut QuectoWorld, qualified: String, name: String, window: u32) {
    ucfg_write(
        world,
        &serde_json::json!({"overrides": {qualified: {"name": name, "contextWindow": window}}}),
    );
}

#[given(expr = "a user catalogue file overriding model {string} with a literal secret {string}")]
fn ucfg_given_override_literal_secret(world: &mut QuectoWorld, qualified: String, secret: String) {
    ucfg_write(
        world,
        &serde_json::json!({"overrides": {qualified: {"apiKey": secret}}}),
    );
}

#[given(
    expr = "a user catalogue file adding provider {string} on transport {string} with model {string}"
)]
fn ucfg_given_provider_add(
    world: &mut QuectoWorld,
    provider: String,
    transport: String,
    model: String,
) {
    ucfg_write(
        world,
        &serde_json::json!({"providers": {provider.clone(): {
            "api": transport,
            "models": [{"id": model}]
        }}}),
    );
    world.catalogue_user_config.provider_under_build = Some(provider);
}

fn ucfg_patch_provider(world: &mut QuectoWorld, field: &str, value: serde_json::Value) {
    let provider = world
        .catalogue_user_config
        .provider_under_build
        .clone()
        .expect("an 'adding provider' Given must come first");
    let mut file = ucfg_read(world);
    file["providers"][&provider][field] = value;
    ucfg_write(world, &file);
}

#[given(expr = "that provider has base url {string}")]
fn ucfg_given_provider_base_url(world: &mut QuectoWorld, base_url: String) {
    ucfg_patch_provider(world, "baseUrl", serde_json::Value::String(base_url));
}

#[given(expr = "that provider references credential {string}")]
fn ucfg_given_provider_credential_ref(world: &mut QuectoWorld, reference: String) {
    assert!(
        reference.starts_with('$'),
        "catalogue files carry credential references, never literals"
    );
    ucfg_patch_provider(world, "apiKey", serde_json::Value::String(reference));
}

#[given(expr = "the environment provides {string}")]
fn ucfg_given_env(_world: &mut QuectoWorld, name: String) {
    // SAFETY: test-only env mutation with a scenario-specific variable name.
    unsafe { std::env::set_var(name, "bdd-secret-value") };
}

#[given(expr = "the effective catalogue has been resolved from the user's configuration")]
fn ucfg_given_resolved(world: &mut QuectoWorld) {
    ucfg_when_resolved(world);
}

#[given(
    expr = "the user catalogue file has been rewritten to add model {string} named {string} to provider {string}"
)]
fn ucfg_given_rewritten(world: &mut QuectoWorld, id: String, name: String, provider: String) {
    world.catalogue_user_config.snapshot_before_rewrite = world
        .catalogue_user_config
        .resolved
        .as_ref()
        .map(|resolved| resolved.snapshot.clone());
    let mut file = ucfg_read(world);
    file["providers"][&provider]["models"]
        .as_array_mut()
        .expect("models array")
        .push(serde_json::json!({"id": id, "name": name}));
    ucfg_write(world, &file);
}

#[given(expr = "the user catalogue file has been rewritten to malformed JSON")]
fn ucfg_given_rewritten_malformed(world: &mut QuectoWorld) {
    let base = ucfg_base(world);
    std::fs::write(base.join("models.json"), "{not json").expect("write models.json");
}

#[given(expr = "a legacy user models file declaring provider {string} with model {string}")]
fn ucfg_given_legacy(world: &mut QuectoWorld, provider: String, model: String) {
    // The historical models.json shape: baseUrl + models, nothing new.
    ucfg_write(
        world,
        &serde_json::json!({"providers": {provider: {
            "api": "openai-completions",
            "baseUrl": "https://legacy.example/v1",
            "models": [{"id": model}]
        }}}),
    );
}

#[given(
    expr = "a legacy user models file declaring provider {string} with model {string} and a literal apiKey"
)]
fn ucfg_given_legacy_literal_key(world: &mut QuectoWorld, provider: String, model: String) {
    // The AC5/AC6 boundary: the legacy provider-level literal apiKey stays
    // accepted for compatibility; only the overrides surface is
    // reference-only (documented in docs/runtime-models-providers.md).
    ucfg_write(
        world,
        &serde_json::json!({"providers": {provider: {
            "api": "openai-completions",
            "baseUrl": "https://legacy.example/v1",
            "apiKey": "legacy-literal-key",
            "models": [{"id": model}]
        }}}),
    );
}

#[when(expr = "the effective catalogue is resolved from the user's configuration")]
fn ucfg_when_resolved(world: &mut QuectoWorld) {
    let base = ucfg_base(world);
    let (_store, resolved) = resolve_and_publish_for(&base);
    world.catalogue_user_config.resolved = Some(resolved);
}

#[when(expr = "the UDS models listing is requested")]
fn ucfg_when_uds_listing(world: &mut QuectoWorld) {
    let base = ucfg_base(world);
    world.catalogue_user_config.uds_listing = Some(list_models_data(&base));
}

#[then(expr = "the published snapshot lists model {string} named {string}")]
fn ucfg_then_listed_named(world: &mut QuectoWorld, qualified: String, name: String) {
    let entry = ucfg_find(ucfg_resolved(world), &qualified);
    assert_eq!(entry.model.display_name.as_deref(), Some(name.as_str()));
}

#[then(expr = "the published snapshot lists model {string}")]
#[then(expr = "the published snapshot still lists model {string}")]
fn ucfg_then_listed(world: &mut QuectoWorld, qualified: String) {
    let reference = ModelRef::parse_qualified(&qualified).expect("qualified id");
    assert!(
        ucfg_resolved(world).snapshot.find(&reference).is_some(),
        "model '{qualified}' not in published snapshot"
    );
}

#[then(expr = "the published model {string} has context window {int}")]
fn ucfg_then_context_window(world: &mut QuectoWorld, qualified: String, window: u32) {
    let entry = ucfg_find(ucfg_resolved(world), &qualified);
    assert_eq!(entry.model.capabilities.context_window, window);
}

#[then(expr = "the published snapshot lists model {string} exactly once")]
fn ucfg_then_listed_once(world: &mut QuectoWorld, qualified: String) {
    let count = ucfg_resolved(world)
        .snapshot
        .entries()
        .iter()
        .filter(|entry| entry.reference().qualified_id() == qualified)
        .count();
    assert_eq!(count, 1, "'{qualified}' must appear exactly once");
}

#[then(expr = "the published model {string} is runnable")]
fn ucfg_then_runnable(world: &mut QuectoWorld, qualified: String) {
    let entry = ucfg_find(ucfg_resolved(world), &qualified);
    assert!(
        entry.model.availability.is_runnable(),
        "'{qualified}' must be runnable, got {:?}",
        entry.model.availability
    );
}

#[then(expr = "the published model {string} is not runnable because its transport is unsupported")]
fn ucfg_then_unsupported(world: &mut QuectoWorld, qualified: String) {
    let entry = ucfg_find(ucfg_resolved(world), &qualified);
    assert!(!entry.model.availability.is_runnable());
    assert!(
        entry
            .model
            .availability
            .reasons()
            .iter()
            .any(|reason| matches!(reason, UnavailableReason::UnsupportedTransport { .. })),
        "expected a structured unsupported-transport reason, got {:?}",
        entry.model.availability.reasons()
    );
}

#[then(expr = "the snapshot resolved before the rewrite does not list model {string}")]
fn ucfg_then_absent_before_rewrite(world: &mut QuectoWorld, qualified: String) {
    let before = world
        .catalogue_user_config
        .snapshot_before_rewrite
        .as_ref()
        .expect("a snapshot was resolved before the rewrite");
    assert!(
        before
            .find(&ModelRef::parse_qualified(&qualified).expect("qualified id"))
            .is_none(),
        "'{qualified}' must only appear after the file edit was re-read"
    );
}

#[then(expr = "the resolution reports a user catalogue error mentioning {string}")]
fn ucfg_then_error_mentions(world: &mut QuectoWorld, fragment: String) {
    let resolved = ucfg_resolved(world);
    let mentioned = resolved
        .source_errors
        .iter()
        .map(|error| &error.error)
        .chain(resolved.skipped.iter().map(|(_, skipped)| &skipped.error))
        .any(|error| error.to_lowercase().contains(&fragment.to_lowercase()));
    assert!(
        mentioned,
        "expected an error mentioning '{fragment}', got source_errors={:?} skipped={:?}",
        resolved.source_errors, resolved.skipped
    );
}

#[then(expr = "the published model {string} keeps its built-in name")]
fn ucfg_then_keeps_builtin_name(world: &mut QuectoWorld, qualified: String) {
    let entry = ucfg_find(ucfg_resolved(world), &qualified);
    // The built-in display name for openai-api/gpt-5.5 in the registry table.
    assert_eq!(
        entry.model.display_name.as_deref(),
        Some("GPT 5.5 (API key)"),
        "a rejected override must leave the built-in metadata untouched"
    );
}

#[then(expr = "the UDS models listing includes model {string}")]
fn ucfg_then_uds_lists(world: &mut QuectoWorld, qualified: String) {
    let listing = world
        .catalogue_user_config
        .uds_listing
        .as_ref()
        .expect("UDS models listing requested");
    let listed = listing["models"]
        .as_array()
        .expect("models array")
        .iter()
        .any(|model| model["model"] == qualified);
    assert!(
        listed,
        "UDS listing must include '{qualified}', got {listing}"
    );
}
