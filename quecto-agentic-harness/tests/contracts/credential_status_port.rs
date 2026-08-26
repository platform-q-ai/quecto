//! Contract for the application `CredentialStatusPort` (issue #1572): the
//! port answers availability as a boolean only — credential material never
//! crosses it.

use quecto::application::ports::CredentialStatusPort;
use quecto::domain::catalogue::{AuthIdentity, ProviderDescriptor, ProviderId, TransportKind};
use quecto::infrastructure::catalogue_registry::RegistryCredentialStatus;
use quecto::infrastructure::model_registry::ModelRegistry;

fn descriptor(id: &str) -> ProviderDescriptor {
    ProviderDescriptor {
        id: ProviderId::new(id).unwrap(),
        display_name: None,
        transport: TransportKind::OpenAiCompletions,
        auth: AuthIdentity::ApiKey,
    }
}

#[test]
fn registry_adapter_matches_the_legacy_configured_predicate() {
    let builtin = ModelRegistry::builtin();
    let mut keyed = builtin.models()[0].clone();
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
    assert!(status.credential_available(&descriptor("keyed")));
    assert!(status.credential_available(&descriptor("urled")));
    assert!(!status.credential_available(&descriptor("bare")));
    assert!(!status.credential_available(&descriptor("unknown")));
}
