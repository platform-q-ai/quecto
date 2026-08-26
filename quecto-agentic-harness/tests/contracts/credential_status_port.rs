//! Contract for the application `CredentialStatusPort` (issue #1572): the
//! port answers availability as a boolean only — credential material never
//! crosses it.

use quecto::application::ports::CredentialStatusPort;
use quecto::domain::catalogue::{
    AuthIdentity, Availability, CatalogueEntry, ModelCapabilities, ModelCost, ModelDescriptor,
    ModelRef, ProviderDescriptor, ProviderId, TransportKind,
};
use quecto::infrastructure::catalogue_registry::RegistryCredentialStatus;
use quecto::infrastructure::model_registry::ModelRegistry;

fn catalogue_entry(provider: &str, model: &str) -> CatalogueEntry {
    let reference = ModelRef::parse(provider, model).unwrap();
    CatalogueEntry {
        provider: ProviderDescriptor {
            id: ProviderId::new(provider).unwrap(),
            display_name: None,
            transport: TransportKind::OpenAiCompletions,
            auth: AuthIdentity::ApiKey,
        },
        model: ModelDescriptor {
            reference,
            display_name: None,
            capabilities: ModelCapabilities {
                input_modalities: vec!["text".to_string()],
                context_window: 128_000,
                max_output_tokens: 4096,
                context_window_explicit: false,
                max_output_tokens_explicit: false,
                reasoning: false,
                cost: ModelCost::default(),
            },
            availability: Availability::runnable(),
        },
    }
}

#[test]
fn registry_adapter_matches_the_legacy_configured_predicate() {
    let builtin = ModelRegistry::builtin();
    let mut keyed = builtin.models()[0].clone();
    let model_id = keyed.id.clone();
    keyed.provider = "keyed".to_string();
    keyed.api_key = Some("sk-live".to_string());
    let mut urled = keyed.clone();
    urled.provider = "urled".to_string();
    urled.api_key = None;
    urled.base_url = Some("https://example.test/v1".to_string());
    let mut bare = keyed.clone();
    bare.provider = "bare".to_string();
    bare.api_key = Some(String::new());
    bare.base_url = None;

    let status = RegistryCredentialStatus::from_records([&keyed, &urled, &bare]);
    assert!(status.credential_available(&catalogue_entry("keyed", &model_id)));
    assert!(status.credential_available(&catalogue_entry("urled", &model_id)));
    assert!(!status.credential_available(&catalogue_entry("bare", &model_id)));
    assert!(!status.credential_available(&catalogue_entry("unknown", &model_id)));
}

/// Legacy `configured` was a per-record predicate: a key resolved onto one
/// model must not mark sibling models of the same provider configured.
#[test]
fn credential_status_is_per_record_not_per_provider() {
    let builtin = ModelRegistry::builtin();
    let mut keyed = builtin.models()[0].clone();
    keyed.provider = "keyed".to_string();
    keyed.api_key = Some("sk-live".to_string());

    let status = RegistryCredentialStatus::from_records([&keyed]);
    assert!(status.credential_available(&catalogue_entry("keyed", &keyed.id)));
    assert!(
        !status.credential_available(&catalogue_entry("keyed", "sibling-without-key")),
        "a key on one record must not configure provider siblings"
    );
}
