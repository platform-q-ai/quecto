//! Tests for the effective-catalogue interface bridge (issue #1572).

use super::*;

#[test]
fn resolve_and_publish_for_publishes_one_generation_per_call() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, resolved) = resolve_and_publish_for(tmp.path());
    assert!(resolved.source_errors.is_empty());
    let first = store.current().generation();
    assert!(first >= 1);
    let (_, resolved_again) = resolve_and_publish_for(tmp.path());
    assert_eq!(resolved_again.snapshot.generation(), first + 1);
    // Same base_dir shares one store.
    assert_eq!(
        snapshot_store_for(tmp.path()).current().generation(),
        first + 1
    );
}

#[test]
fn unqualified_model_ids_have_no_limits() {
    let tmp = tempfile::tempdir().unwrap();
    let (cap, window) = model_limits_from_base_dir(tmp.path(), "not-qualified");
    assert_eq!(cap, None);
    assert_eq!(window, None);
}

#[test]
fn model_limits_from_base_dir_reads_output_cap_from_models_json() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"qwen3p7-plus","maxTokens":65536}]}}}"#,
    )
    .unwrap();

    let (cap, window) = model_limits_from_base_dir(tmp.path(), "fireworks/qwen3p7-plus");
    assert_eq!(cap, Some(65_536));
    assert_eq!(window, None, "no declared window must not clamp");
}

#[test]
fn model_limits_from_base_dir_reads_context_window_from_models_json() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"small-window","contextWindow":32768},{"id":"no-window"}]}}}"#,
    )
    .unwrap();

    assert_eq!(
        model_limits_from_base_dir(tmp.path(), "fireworks/small-window").1,
        Some(32_768)
    );
    assert_eq!(
        model_limits_from_base_dir(tmp.path(), "fireworks/no-window").1,
        None,
        "a listed model without a declared window must not clamp"
    );
}

#[test]
fn model_limits_survive_a_malformed_models_json_via_the_builtin_layer() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("models.json"), "not json").unwrap();
    // Malformed-source isolation: the built-in layer still resolves, so a
    // declared builtin window keeps clamping.
    assert_eq!(
        model_limits_from_base_dir(tmp.path(), "anthropic-api/claude-sonnet-5").1,
        Some(1_000_000)
    );
}

/// Slice-4 review: a discovered-cache model under a provider configured with
/// only auth + baseUrl (no listed models) must be credentialed and routable —
/// the legacy discover flow guaranteed this by rewriting models.json, and the
/// cache-only flow must not lose it.
#[test]
fn discovered_models_inherit_provider_credentials_and_join_the_effective_registry() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        serde_json::json!({"providers": {
            "openrouter": {
                "api": "openai-completions",
                "baseUrl": "https://openrouter.example/v1",
                "apiKey": "sk-or-key",
                "models": []
            }
        }})
        .to_string(),
    )
    .unwrap();
    crate::infrastructure::catalogue_discovery::DiscoverySourceCache::new(
        &crate::infrastructure::catalogue_discovery::discovery_cache_dir(tmp.path()),
        "openrouter",
    )
    .store_models_response(r#"{"data":[{"id":"alpha","name":"Alpha"}]}"#)
    .unwrap();

    let (_store, resolved) = resolve_and_publish_for(tmp.path());
    let entry = resolved
        .snapshot
        .find(&crate::domain::catalogue::ModelRef::parse_qualified("openrouter/alpha").unwrap())
        .expect("discovered model must be published");
    assert!(
        entry.model.availability.is_runnable(),
        "a discovered model under a credentialed provider must be runnable, got {:?}",
        entry.model.availability
    );

    let registry = CatalogueInputs::load(tmp.path())
        .effective_registry()
        .expect("registry must build");
    let record = registry
        .find("openrouter", "alpha")
        .expect("discovered model must have an effective-registry record (a route)");
    assert_eq!(record.api_key.as_deref(), Some("sk-or-key"));
    assert_eq!(
        record.base_url.as_deref(),
        Some("https://openrouter.example/v1")
    );
}

/// A model the user lists explicitly keeps its own record even when the
/// discovery cache also carries it (the file wins over synthesis).
#[test]
fn user_listed_models_win_over_synthesized_discovered_records() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        serde_json::json!({"providers": {
            "openrouter": {
                "api": "openai-completions",
                "baseUrl": "https://openrouter.example/v1",
                "apiKey": "sk-or-key",
                "models": [{"id": "alpha", "name": "Mine", "maxTokens": 999}]
            }
        }})
        .to_string(),
    )
    .unwrap();
    crate::infrastructure::catalogue_discovery::DiscoverySourceCache::new(
        &crate::infrastructure::catalogue_discovery::discovery_cache_dir(tmp.path()),
        "openrouter",
    )
    .store_models_response(r#"{"data":[{"id":"alpha","name":"Theirs"}]}"#)
    .unwrap();

    let registry = CatalogueInputs::load(tmp.path())
        .effective_registry()
        .expect("registry must build");
    let record = registry.find("openrouter", "alpha").expect("record");
    assert_eq!(record.display_name.as_deref(), Some("Mine"));
    assert_eq!(record.max_tokens, 999);
}

// --- issue #1575 (epic #1193, slice 5): user-owned extension surface -------

fn slice5_write(tmp: &tempfile::TempDir, json: &str) {
    std::fs::write(tmp.path().join("models.json"), json).unwrap();
}

const SLICE5_MIXED_TRANSPORTS: &str = r#"{"providers":{
    "custom":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"$CUSTOM_KEY","models":[{"id":"m1"}]},
    "wsprov":{"api":"websocket-frames","models":[{"id":"m2"}]}
}}"#;

/// AC3 (part): a valid provider must survive an unsupported-transport
/// neighbour in the same user file instead of the whole layer erroring away.
#[test]
fn valid_provider_survives_an_unsupported_transport_neighbour() {
    use crate::domain::catalogue::ModelRef;
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(&tmp, SLICE5_MIXED_TRANSPORTS);
    let (_store, resolved) = resolve_and_publish_for(tmp.path());
    let good = ModelRef::parse_qualified("custom/m1").unwrap();
    assert!(
        resolved.snapshot.find(&good).is_some(),
        "a valid sibling provider must survive an unsupported-transport neighbour"
    );
}

/// AC3 (part): the unsupported-transport entry itself is listed as known.
#[test]
fn unsupported_transport_entry_is_listed_as_known() {
    use crate::domain::catalogue::ModelRef;
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(&tmp, SLICE5_MIXED_TRANSPORTS);
    let (_store, resolved) = resolve_and_publish_for(tmp.path());
    let unsupported = ModelRef::parse_qualified("wsprov/m2").unwrap();
    assert!(
        resolved.snapshot.find(&unsupported).is_some(),
        "unsupported-transport entry must be listed as known"
    );
}

/// AC3 (part): the listed entry is not runnable and carries a structured
/// unsupported-transport reason.
#[test]
fn unsupported_transport_entry_is_not_runnable_with_structured_reason() {
    use crate::domain::catalogue::{ModelRef, UnavailableReason};
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(&tmp, SLICE5_MIXED_TRANSPORTS);
    let (_store, resolved) = resolve_and_publish_for(tmp.path());
    let unsupported = ModelRef::parse_qualified("wsprov/m2").unwrap();
    let entry = resolved
        .snapshot
        .find(&unsupported)
        .expect("unsupported-transport entry must be listed as known");
    assert!(!entry.model.availability.is_runnable());
    assert!(
        entry
            .model
            .availability
            .reasons()
            .iter()
            .any(|r| matches!(r, UnavailableReason::UnsupportedTransport { .. })),
        "expected a structured unsupported-transport reason, got: {:?}",
        entry.model.availability.reasons()
    );
}

const SLICE5_OVERRIDE: &str =
    r#"{"overrides":{"openai-api/gpt-5.5":{"name":"My 5.5","contextWindow":999000}}}"#;

/// AC1 (part): a stable-ID override replaces the built-in display name.
#[test]
fn stable_id_override_replaces_builtin_display_name() {
    use crate::domain::catalogue::ModelRef;
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(&tmp, SLICE5_OVERRIDE);
    let (_store, resolved) = resolve_and_publish_for(tmp.path());
    let reference = ModelRef::parse_qualified("openai-api/gpt-5.5").unwrap();
    let entry = resolved.snapshot.find(&reference).expect("builtin entry");
    assert_eq!(
        entry.model.display_name.as_deref(),
        Some("My 5.5"),
        "override by stable ID must replace the built-in display name"
    );
}

/// AC1 (part): a stable-ID override replaces the built-in context window.
#[test]
fn stable_id_override_replaces_builtin_context_window() {
    use crate::domain::catalogue::ModelRef;
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(&tmp, SLICE5_OVERRIDE);
    let (_store, resolved) = resolve_and_publish_for(tmp.path());
    let reference = ModelRef::parse_qualified("openai-api/gpt-5.5").unwrap();
    let entry = resolved.snapshot.find(&reference).expect("builtin entry");
    assert_eq!(
        entry.model.capabilities.context_window, 999_000,
        "override by stable ID must replace the built-in context window"
    );
}

/// AC5: catalogue files carry credential *references*, never literal secrets.
/// A literal-secret field in the override surface must be rejected with a
/// clear, structured error rather than silently accepted or ignored.
#[test]
fn literal_secret_in_override_surface_is_rejected_with_structured_error() {
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(
        &tmp,
        r#"{"overrides":{"openai-api/gpt-5.5":{"apiKey":"sk-live-secret123"}}}"#,
    );
    let (_store, resolved) = resolve_and_publish_for(tmp.path());
    let mentions = |text: &str| text.contains("credential reference");
    assert!(
        resolved.source_errors.iter().any(|e| mentions(&e.error))
            || resolved.skipped.iter().any(|(_, s)| mentions(&s.error)),
        "a literal secret in an override must produce an error naming credential references; got source_errors={:?} skipped={:?}",
        resolved.source_errors,
        resolved.skipped
    );
}

/// AC1a: a data-only model add on an existing provider is published with its
/// declared display name.
#[test]
fn user_file_model_add_on_existing_provider_is_published() {
    use crate::domain::catalogue::ModelRef;
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(
        &tmp,
        r#"{"providers":{"openai-api":{"api":"openai-completions","models":[{"id":"gpt-5.5-preview","name":"GPT 5.5 Preview"}]}}}"#,
    );
    let (_store, resolved) = resolve_and_publish_for(tmp.path());
    let added = ModelRef::parse_qualified("openai-api/gpt-5.5-preview").unwrap();
    let entry = resolved.snapshot.find(&added).expect("added model listed");
    assert_eq!(entry.model.display_name.as_deref(), Some("GPT 5.5 Preview"));
}

/// AC2: a data-only provider add on an existing transport reaches runnable
/// with a base url and a credential reference resolved from the environment.
#[test]
fn user_file_provider_add_with_credential_reference_is_runnable() {
    use crate::domain::catalogue::ModelRef;
    // SAFETY: test-only env mutation with a name no other test reads.
    unsafe { std::env::set_var("SLICE5_GATEWAY_KEY", "gw-secret") };
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(
        &tmp,
        r#"{"providers":{"my-gateway":{"api":"openai-completions","baseUrl":"https://gw.example/v1","apiKey":"$SLICE5_GATEWAY_KEY","models":[{"id":"custom-model"}]}}}"#,
    );
    let (_store, resolved) = resolve_and_publish_for(tmp.path());
    let added = ModelRef::parse_qualified("my-gateway/custom-model").unwrap();
    let entry = resolved.snapshot.find(&added).expect("added model listed");
    assert!(
        entry.model.availability.is_runnable(),
        "a credentialed provider add on a supported transport must be runnable, got {:?}",
        entry.model.availability
    );
}

/// AC5/AC6 boundary: the legacy provider-level literal `apiKey` stays
/// accepted for compatibility — only the new `overrides` surface is
/// reference-only (documented in docs/runtime-models-providers.md).
#[test]
fn legacy_provider_level_literal_api_key_stays_accepted() {
    use crate::domain::catalogue::ModelRef;
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(
        &tmp,
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"legacy-literal-key","models":[{"id":"qwen3p7-plus"}]}}}"#,
    );
    let (_store, resolved) = resolve_and_publish_for(tmp.path());
    assert!(resolved.source_errors.is_empty() && resolved.skipped.is_empty());
    let legacy = ModelRef::parse_qualified("fireworks/qwen3p7-plus").unwrap();
    let entry = resolved
        .snapshot
        .find(&legacy)
        .expect("legacy model listed");
    assert!(entry.model.availability.is_runnable());
}

/// AC1b guard: an override targeting an unknown model is a per-record
/// diagnostic, not silently dropped and not a layer failure.
#[test]
fn override_of_unknown_model_is_reported_not_dropped() {
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(&tmp, r#"{"overrides":{"nope/missing":{"name":"X"}}}"#);
    let (_store, resolved) = resolve_and_publish_for(tmp.path());
    assert!(
        resolved
            .skipped
            .iter()
            .any(|(_, s)| s.record == "nope/missing" && s.error.contains("known model")),
        "unknown override target must surface as a diagnostic, got {:?}",
        resolved.skipped
    );
}

/// #1581 review: an override apiKey referencing an unset environment
/// variable must be rejected with a diagnostic and keep the base
/// credential, never silently clobber it with an empty key.
#[test]
fn override_referencing_unset_env_var_is_rejected_and_keeps_base_credential() {
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(
        &tmp,
        r#"{"overrides":{"openai-api/gpt-5.5":{"apiKey":"$QUECTO_TEST_DEFINITELY_UNSET_VAR"}}}"#,
    );
    let (_store, resolved) = resolve_and_publish_for(tmp.path());
    assert!(
        resolved.skipped.iter().any(|(_, s)| {
            s.record == "openai-api/gpt-5.5" && s.error.contains("unset or empty")
        }),
        "an unset credential reference must surface as a diagnostic, got {:?}",
        resolved.skipped
    );
}

/// #1581 review: an override can patch a known-but-unrunnable
/// unsupported-transport declaration (it is a published entry, so it is
/// patchable by stable ID like any other known entry).
#[test]
fn override_patches_an_unsupported_transport_entry() {
    use crate::domain::catalogue::ModelRef;
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(
        &tmp,
        r#"{"providers":{"wsprov":{"api":"websocket-frames","models":[{"id":"m2"}]}},
            "overrides":{"wsprov/m2":{"name":"My WS","contextWindow":42000}}}"#,
    );
    let (_store, resolved) = resolve_and_publish_for(tmp.path());
    let entry = resolved
        .snapshot
        .find(&ModelRef::parse_qualified("wsprov/m2").unwrap())
        .expect("unsupported entry listed");
    assert_eq!(entry.model.display_name.as_deref(), Some("My WS"));
    assert_eq!(entry.model.capabilities.context_window, 42_000);
    assert!(
        !entry.model.availability.is_runnable(),
        "patching metadata must not make an unsupported transport runnable"
    );
    assert!(
        !resolved
            .skipped
            .iter()
            .any(|(_, s)| s.record == "wsprov/m2"),
        "the override must apply, not be rejected: {:?}",
        resolved.skipped
    );
}
