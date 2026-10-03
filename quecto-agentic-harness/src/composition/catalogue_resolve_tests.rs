//! Tests for the effective-catalogue resolve over the real inputs (issue
//! #1572) and the per-model limits the change-active-model use case reads
//! (#935/#1044, #1847), through the composition helpers rigs share.

use crate::composition::catalogue::{published_model_limits_for, resolve_catalogue_for};
use crate::domain::conversation::image_input::ImageInput;
use crate::infrastructure::catalogue_inputs::CatalogueInputs;
use crate::infrastructure::catalogue_registry::snapshot_store_for;

fn published_model_limits(dir: &std::path::Path, model: &str) -> (Option<u32>, Option<usize>) {
    let limits = published_model_limits_for(dir, model);
    (limits.max_output_tokens, limits.context_window)
}

#[test]
fn resolve_publishes_one_generation_per_call() {
    let tmp = tempfile::tempdir().unwrap();
    let resolved = resolve_catalogue_for(tmp.path());
    assert!(resolved.source_errors.is_empty());
    let first = resolved.snapshot.generation();
    assert!(first >= 1);
    let resolved_again = resolve_catalogue_for(tmp.path());
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
    let (cap, window) = published_model_limits(tmp.path(), "not-qualified");
    assert_eq!(cap, None);
    assert_eq!(window, None);
}

#[test]
fn published_model_limits_reads_output_cap_from_models_json() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"qwen3p7-plus","maxTokens":65536}]}}}"#,
    )
    .unwrap();

    let (cap, window) = published_model_limits(tmp.path(), "fireworks/qwen3p7-plus");
    assert_eq!(cap, Some(65_536));
    assert_eq!(window, None, "no declared window must not clamp");
}

#[test]
fn published_model_limits_reads_context_window_from_models_json() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"small-window","contextWindow":32768},{"id":"no-window"}]}}}"#,
    )
    .unwrap();

    assert_eq!(
        published_model_limits(tmp.path(), "fireworks/small-window").1,
        Some(32_768)
    );
    assert_eq!(
        published_model_limits(tmp.path(), "fireworks/no-window").1,
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
        published_model_limits(tmp.path(), "anthropic-api/claude-sonnet-5").1,
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

    let resolved = resolve_catalogue_for(tmp.path());
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
    let resolved = resolve_catalogue_for(tmp.path());
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
    let resolved = resolve_catalogue_for(tmp.path());
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
    let resolved = resolve_catalogue_for(tmp.path());
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
    r#"{"overrides":{"openai-api/gpt-5.6-sol":{"name":"My 5.6 Sol","contextWindow":999000}}}"#;

/// AC1 (part): a stable-ID override replaces the built-in display name.
#[test]
fn stable_id_override_replaces_builtin_display_name() {
    use crate::domain::catalogue::ModelRef;
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(&tmp, SLICE5_OVERRIDE);
    let resolved = resolve_catalogue_for(tmp.path());
    let reference = ModelRef::parse_qualified("openai-api/gpt-5.6-sol").unwrap();
    let entry = resolved.snapshot.find(&reference).expect("builtin entry");
    assert_eq!(
        entry.model.display_name.as_deref(),
        Some("My 5.6 Sol"),
        "override by stable ID must replace the built-in display name"
    );
}

/// AC1 (part): a stable-ID override replaces the built-in context window.
#[test]
fn stable_id_override_replaces_builtin_context_window() {
    use crate::domain::catalogue::ModelRef;
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(&tmp, SLICE5_OVERRIDE);
    let resolved = resolve_catalogue_for(tmp.path());
    let reference = ModelRef::parse_qualified("openai-api/gpt-5.6-sol").unwrap();
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
        r#"{"overrides":{"openai-api/gpt-5.6-sol":{"apiKey":"sk-live-secret123"}}}"#,
    );
    let resolved = resolve_catalogue_for(tmp.path());
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
        r#"{"providers":{"openai-api":{"api":"openai-completions","models":[{"id":"gpt-6.2-preview","name":"GPT 6.2 Preview"}]}}}"#,
    );
    let resolved = resolve_catalogue_for(tmp.path());
    let added = ModelRef::parse_qualified("openai-api/gpt-6.2-preview").unwrap();
    let entry = resolved.snapshot.find(&added).expect("added model listed");
    assert_eq!(entry.model.display_name.as_deref(), Some("GPT 6.2 Preview"));
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
    let resolved = resolve_catalogue_for(tmp.path());
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
    let resolved = resolve_catalogue_for(tmp.path());
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
    let resolved = resolve_catalogue_for(tmp.path());
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
        r#"{"overrides":{"openai-api/gpt-5.6-sol":{"apiKey":"$QUECTO_TEST_DEFINITELY_UNSET_VAR"}}}"#,
    );
    let resolved = resolve_catalogue_for(tmp.path());
    assert!(
        resolved.skipped.iter().any(|(_, s)| {
            s.record == "openai-api/gpt-5.6-sol" && s.error.contains("unset or empty")
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
    let resolved = resolve_catalogue_for(tmp.path());
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

/// #2421: what the change-active-model use case reads a model takes of a
/// conversation's images, over the real inputs in `dir`.
fn image_input(dir: &std::path::Path, model: &str) -> ImageInput {
    published_model_limits_for(dir, model).image_input
}

/// #2421: the built-in vision models take images: every image over the
/// Anthropic wire, still images over the OpenAI one; a model the catalogue
/// does not list (Codex Spark, retired in #2435) none.
#[test]
fn builtin_vision_models_take_images() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(
        image_input(tmp.path(), "anthropic-api/claude-opus-5"),
        ImageInput::AllImages
    );
    assert_eq!(
        image_input(tmp.path(), "openai-oauth/gpt-6.1-sol"),
        ImageInput::StillImages
    );
    assert_eq!(
        image_input(tmp.path(), "openai-oauth/gpt-5.3-codex-spark"),
        ImageInput::NoImages
    );
}

/// #2421 review M1: an override's `input` turns images off for a built-in
/// model without redeclaring its provider, or on for a model the user's
/// file declares text-only (a retired built-in, #2435).
#[test]
fn an_override_patches_a_builtin_models_input() {
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(
        &tmp,
        r#"{"providers":{"openai-oauth":{"auth":{"mode":"oauth","oauthProvider":"openai"},
              "models":[{"id":"gpt-5.3-codex-spark"}]}},
            "overrides":{
            "openai-oauth/gpt-6.1-sol":{"input":["text"]},
            "openai-oauth/gpt-5.3-codex-spark":{"input":["text","image"]}
        }}"#,
    );
    assert_eq!(
        image_input(tmp.path(), "openai-oauth/gpt-6.1-sol"),
        ImageInput::NoImages
    );
    assert_eq!(
        image_input(tmp.path(), "openai-oauth/gpt-5.3-codex-spark"),
        ImageInput::StillImages
    );
    assert_eq!(
        image_input(tmp.path(), "openai-api/gpt-6.1-sol"),
        ImageInput::StillImages,
        "only the overridden id changes"
    );
}

/// #2421 review L1: a discovered listing says nothing of a model's input,
/// so a built-in model it lists keeps the built-in input.
#[test]
fn a_discovered_builtin_model_keeps_its_builtin_input() {
    let tmp = tempfile::tempdir().unwrap();
    crate::infrastructure::catalogue_discovery::DiscoverySourceCache::new(
        &crate::infrastructure::catalogue_discovery::discovery_cache_dir(tmp.path()),
        "openai-api",
    )
    .store_models_response(r#"{"data":[{"id":"gpt-6.1-sol"},{"id":"gpt-new"}]}"#)
    .unwrap();
    assert_eq!(
        image_input(tmp.path(), "openai-api/gpt-6.1-sol"),
        ImageInput::StillImages
    );
    assert_eq!(
        image_input(tmp.path(), "openai-api/gpt-new"),
        ImageInput::NoImages,
        "a model the built-in table does not know declares text only"
    );
}

/// #2421 review L1: a models.json block listing a built-in model without
/// `input` keeps the built-in input; an explicit `input` wins.
#[test]
fn a_user_listed_builtin_model_without_input_keeps_its_builtin_input() {
    let tmp = tempfile::tempdir().unwrap();
    slice5_write(
        &tmp,
        r#"{"providers":{"anthropic-api":{"api":"anthropic-messages","apiKey":"$ANTHROPIC_API_KEY",
            "models":[{"id":"claude-opus-5"},{"id":"claude-opus-4-8","input":["text"]}]}}}"#,
    );
    assert_eq!(
        image_input(tmp.path(), "anthropic-api/claude-opus-5"),
        ImageInput::AllImages
    );
    assert_eq!(
        image_input(tmp.path(), "anthropic-api/claude-opus-4-8"),
        ImageInput::NoImages
    );
}

/// #2421 review L2: the provider is matched whatever its case. Here no
/// runtime is published, so no router says where a bare id goes: a bare id
/// reads no entry and takes no image (round 3 L2, fail closed).
#[test]
fn a_model_in_another_case_reads_its_builtin_input_and_a_bare_one_none() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(
        image_input(tmp.path(), "OpenAI-OAuth/gpt-6.1-sol"),
        ImageInput::StillImages
    );
    assert_eq!(image_input(tmp.path(), "gpt-6.1-sol"), ImageInput::NoImages);
    assert_eq!(
        image_input(tmp.path(), "claude-opus-5"),
        ImageInput::NoImages
    );
}
