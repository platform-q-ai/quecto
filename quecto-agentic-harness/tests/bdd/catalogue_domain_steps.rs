//! Steps for catalogue_domain.feature (issue #1571, epic #1193 slice 1).
//!
//! Exercises the pure domain catalogue model only: typed ids, provider
//! identity, layer precedence, validation, and availability reasons.

use super::*;
use quecto::domain::catalogue::{
    AuthIdentity, Availability, AvailabilityStatus, CatalogueDomainError, CatalogueEntry,
    CatalogueResolution, ModelCapabilities, ModelCost, ModelDescriptor, ModelRef,
    ProviderDescriptor, ProviderId, SourceLayer, TransportKind, UnavailableReason,
    resolve_catalogue,
};

#[derive(Debug, Default)]
pub struct CatalogueDomainState {
    pub qualified_input: String,
    pub parsed_ref: Option<Result<ModelRef, CatalogueDomainError>>,
    pub id_result: Option<Result<ProviderId, CatalogueDomainError>>,
    pub descriptors: Vec<ProviderDescriptor>,
    pub layers: Vec<(SourceLayer, Vec<CatalogueEntry>)>,
    pub resolution: Option<CatalogueResolution>,
    pub identities_same: Option<bool>,
    pub availability_result: Option<Result<Availability, CatalogueDomainError>>,
}

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        effort_levels: Vec::new(),
        input_modalities: vec!["text".to_string()],
        context_window: 128_000,
        max_output_tokens: 4096,
        context_window_explicit: true,
        max_output_tokens_explicit: false,
        reasoning: false,
        cost: ModelCost::default(),
    }
}

fn make_descriptor(id: &str, auth: AuthIdentity) -> ProviderDescriptor {
    ProviderDescriptor {
        id: ProviderId::new(id).unwrap(),
        display_name: Some(id.to_string()),
        transport: TransportKind::OpenAiCompletions,
        auth,
    }
}

fn make_entry(provider: &str, model: &str, display: &str) -> CatalogueEntry {
    make_entry_with_availability(provider, model, display, Availability::runnable())
}

fn make_entry_with_availability(
    provider: &str,
    model: &str,
    display: &str,
    availability: Availability,
) -> CatalogueEntry {
    CatalogueEntry {
        provider: make_descriptor(provider, AuthIdentity::ApiKey),
        model: ModelDescriptor {
            reference: ModelRef::parse(provider, model).unwrap(),
            display_name: Some(display.to_string()),
            capabilities: capabilities(),
            availability,
        },
    }
}

fn layer_named(name: &str) -> SourceLayer {
    match name {
        "built-in" => SourceLayer::BuiltIn,
        "generated" => SourceLayer::Generated,
        "discovered" => SourceLayer::Discovered,
        "extension" => SourceLayer::Extension,
        "user-defined" => SourceLayer::UserDefined,
        "user-override" => SourceLayer::UserOverride,
        other => panic!("unknown source layer '{other}'"),
    }
}

#[given(expr = "the qualified model identifier {string}")]
fn given_qualified_identifier(world: &mut QuectoWorld, value: String) {
    world.catalogue.qualified_input = value;
}

#[when(expr = "I parse it into a typed model reference")]
fn when_parse_reference(world: &mut QuectoWorld) {
    let input = world.catalogue.qualified_input.clone();
    world.catalogue.parsed_ref = Some(ModelRef::parse_qualified(&input));
}

#[then(expr = "the reference serializes back to exactly {string}")]
fn then_reference_round_trips(world: &mut QuectoWorld, expected: String) {
    let parsed = world
        .catalogue
        .parsed_ref
        .as_ref()
        .expect("no parsed reference")
        .as_ref()
        .expect("reference failed to parse");
    assert_eq!(parsed.qualified_id(), expected);
}

#[then(expr = "the reference names provider {string} and model {string}")]
fn then_reference_names(world: &mut QuectoWorld, provider: String, model: String) {
    let parsed = world
        .catalogue
        .parsed_ref
        .as_ref()
        .expect("no parsed reference")
        .as_ref()
        .expect("reference failed to parse");
    assert_eq!(parsed.provider().as_str(), provider);
    assert_eq!(parsed.model().as_str(), model);
}

#[when(expr = "I try to construct a provider id from {string}")]
fn when_construct_provider_id(world: &mut QuectoWorld, value: String) {
    world.catalogue.id_result = Some(ProviderId::new(value));
}

#[then(expr = "the catalogue id construction is rejected")]
fn then_id_rejected(world: &mut QuectoWorld) {
    let result = world.catalogue.id_result.as_ref().expect("no id result");
    assert!(result.is_err(), "expected id construction to be rejected");
}

#[given(expr = "a provider descriptor {string} authenticating with an API key")]
fn given_api_key_descriptor(world: &mut QuectoWorld, id: String) {
    world
        .catalogue
        .descriptors
        .push(make_descriptor(&id, AuthIdentity::ApiKey));
}

#[given(expr = "a provider descriptor {string} authenticating with OAuth via {string}")]
fn given_oauth_descriptor(world: &mut QuectoWorld, id: String, credential_provider: String) {
    world.catalogue.descriptors.push(make_descriptor(
        &id,
        AuthIdentity::OAuth {
            provider: Some(ProviderId::new(credential_provider).unwrap()),
        },
    ));
}

#[when(expr = "I compare the two provider identities")]
fn when_compare_identities(world: &mut QuectoWorld) {
    let [a, b] = &world.catalogue.descriptors[..] else {
        panic!("expected exactly two provider descriptors");
    };
    world.catalogue.identities_same = Some(a.same_identity(b));
}

#[then(expr = "the two provider identities are distinct")]
fn then_identities_distinct(world: &mut QuectoWorld) {
    let same = world
        .catalogue
        .identities_same
        .expect("identities were not compared");
    assert!(!same, "identities unexpectedly equal");
}

#[given(expr = "a {word} catalogue layer defining model {string} named {string}")]
fn given_layer_with_model(world: &mut QuectoWorld, layer: String, reference: String, name: String) {
    let reference = ModelRef::parse_qualified(&reference).unwrap();
    let entry = make_entry(
        reference.provider().as_str(),
        reference.model().as_str(),
        &name,
    );
    world
        .catalogue
        .layers
        .push((layer_named(&layer), vec![entry]));
}

#[given(expr = "a built-in catalogue layer entry whose model reference names a different provider")]
fn given_invalid_layer_entry(world: &mut QuectoWorld) {
    let mut entry = make_entry("openai-api", "bad", "Bad");
    entry.provider = make_descriptor("mismatch", AuthIdentity::ApiKey);
    world
        .catalogue
        .layers
        .push((SourceLayer::BuiltIn, vec![entry]));
}

#[given(expr = "a catalogue entry for {string} that is configured but missing a credential")]
fn given_unavailable_entry(world: &mut QuectoWorld, reference: String) {
    let reference = ModelRef::parse_qualified(&reference).unwrap();
    let availability = Availability::unavailable(
        AvailabilityStatus::Configured,
        vec![UnavailableReason::MissingCredential],
    )
    .unwrap();
    let entry = make_entry_with_availability(
        reference.provider().as_str(),
        reference.model().as_str(),
        "Local",
        availability,
    );
    world
        .catalogue
        .layers
        .push((SourceLayer::UserDefined, vec![entry]));
}

#[when(expr = "I resolve the catalogue layers into a snapshot")]
fn when_resolve_layers(world: &mut QuectoWorld) {
    let layers = std::mem::take(&mut world.catalogue.layers);
    world.catalogue.resolution = Some(resolve_catalogue(1, layers));
}

fn resolved(world: &QuectoWorld) -> &CatalogueResolution {
    world
        .catalogue
        .resolution
        .as_ref()
        .expect("catalogue not resolved")
}

#[then(expr = "the resolved snapshot has {int} model(s)")]
fn then_snapshot_len(world: &mut QuectoWorld, count: usize) {
    assert_eq!(resolved(world).snapshot.entries().len(), count);
}

#[then(expr = "the resolved model {string} is named {string}")]
fn then_resolved_model_named(world: &mut QuectoWorld, reference: String, name: String) {
    let reference = ModelRef::parse_qualified(&reference).unwrap();
    let entry = resolved(world)
        .snapshot
        .find(&reference)
        .expect("model not found in snapshot");
    assert_eq!(entry.model.display_name.as_deref(), Some(name.as_str()));
}

#[then(expr = "{int} catalogue entry was rejected")]
fn then_rejected_count(world: &mut QuectoWorld, count: usize) {
    assert_eq!(resolved(world).rejected.len(), count);
}

#[then(expr = "the resolved model {string} is not runnable because a credential is missing")]
fn then_model_missing_credential(world: &mut QuectoWorld, reference: String) {
    let reference = ModelRef::parse_qualified(&reference).unwrap();
    let entry = resolved(world)
        .snapshot
        .find(&reference)
        .expect("model not found in snapshot");
    assert_eq!(
        entry.model.availability.status(),
        AvailabilityStatus::Configured
    );
    assert!(!entry.model.availability.is_runnable());
    assert_eq!(
        entry.model.availability.reasons(),
        &[UnavailableReason::MissingCredential]
    );
}

#[when(expr = "I try to construct a non-runnable availability with no reason")]
fn when_construct_reasonless_availability(world: &mut QuectoWorld) {
    world.catalogue.availability_result =
        Some(Availability::unavailable(AvailabilityStatus::Known, vec![]));
}

#[then(expr = "the availability construction is rejected for lacking a reason")]
fn then_availability_rejected(world: &mut QuectoWorld) {
    let result = world
        .catalogue
        .availability_result
        .as_ref()
        .expect("no availability result");
    assert_eq!(
        result.as_ref().unwrap_err(),
        &CatalogueDomainError::UnavailableWithoutReason
    );
}
