//! Unit tests for application-owned provider runtime composition and model
//! selection (issue #1573, epic #1193 slice 3). RED phase: these compile
//! against the skeleton and fail until the GREEN implementation lands.

use super::*;
use crate::application::catalogue::{CatalogueSnapshotStore, SourceEntries};
use crate::domain::catalogue::{
    AuthIdentity, CatalogueEntry, ModelCapabilities, ModelCost, ModelDescriptor, ModelRef,
    ProviderDescriptor, SourceLayer, TransportKind,
};
use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::ChatRequest;
use std::sync::Mutex;

#[derive(Debug)]
struct FakeProvider {
    name: String,
}

impl LlmProvider for FakeProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>,
    > {
        Box::pin(async { Err(DomainError::Provider("fake".into())) })
    }
}

/// A factory whose result can be swapped mid-test to simulate a composition
/// failure after a valid runtime has been published.
struct FakeFactory {
    result: Mutex<Result<String, String>>,
}

impl FakeFactory {
    fn named(name: &str) -> Self {
        Self {
            result: Mutex::new(Ok(name.to_string())),
        }
    }

    fn fail_with(&self, error: &str) {
        *self.result.lock().unwrap() = Err(error.to_string());
    }
}

impl ProviderRuntimeFactory<(), ()> for FakeFactory {
    fn compose_runtime(&self, _config: &(), _inputs: &()) -> Result<Arc<dyn LlmProvider>, String> {
        self.result
            .lock()
            .unwrap()
            .clone()
            .map(|name| Arc::new(FakeProvider { name }) as Arc<dyn LlmProvider>)
    }
}

struct FakeSource {
    id: String,
    entries: Vec<CatalogueEntry>,
}

impl CatalogueSource for FakeSource {
    fn id(&self) -> &str {
        &self.id
    }
    fn layer(&self) -> SourceLayer {
        SourceLayer::BuiltIn
    }
    fn load(&self) -> Result<SourceEntries, String> {
        Ok(SourceEntries::from(self.entries.clone()))
    }
}

struct FakeCredentials {
    denied: Vec<String>,
}

impl CredentialStatusPort for FakeCredentials {
    fn credential_available(&self, entry: &CatalogueEntry) -> bool {
        !self.denied.iter().any(|p| p == entry.provider.id.as_str())
    }
}

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        input_modalities: vec!["text".to_string()],
        context_window: 128_000,
        max_output_tokens: 4096,
        context_window_explicit: true,
        max_output_tokens_explicit: false,
        reasoning: false,
        cost: ModelCost::default(),
    }
}

fn entry(qualified: &str, display: &str, auth: AuthIdentity) -> CatalogueEntry {
    let reference = ModelRef::parse_qualified(qualified).unwrap();
    CatalogueEntry {
        provider: ProviderDescriptor {
            id: reference.provider().clone(),
            display_name: None,
            transport: TransportKind::OpenAiCompletions,
            auth,
        },
        model: ModelDescriptor {
            reference: reference.clone(),
            display_name: Some(display.to_string()),
            capabilities: capabilities(),
            availability: crate::domain::catalogue::Availability::runnable(),
        },
    }
}

struct Fixture {
    factory: FakeFactory,
    sources: Vec<FakeSource>,
    credentials: FakeCredentials,
    catalogue_store: CatalogueSnapshotStore,
    runtime_store: RuntimeSnapshotStore,
}

impl Fixture {
    fn new(entries: Vec<CatalogueEntry>) -> Self {
        Self {
            factory: FakeFactory::named("router"),
            sources: vec![FakeSource {
                id: "builtin".to_string(),
                entries,
            }],
            credentials: FakeCredentials { denied: Vec::new() },
            catalogue_store: CatalogueSnapshotStore::empty(),
            runtime_store: RuntimeSnapshotStore::new(),
        }
    }

    fn compose(&self) -> Result<ComposedRuntime, RuntimeCompositionError> {
        let refs: Vec<&dyn CatalogueSource> = self
            .sources
            .iter()
            .map(|s| s as &dyn CatalogueSource)
            .collect();
        ComposeProviderRuntimeUseCase::new().compose_and_publish(
            &self.factory,
            &(),
            &(),
            &CompositionPorts {
                sources: &refs,
                credentials: &self.credentials,
                catalogue_store: &self.catalogue_store,
                runtime_store: &self.runtime_store,
            },
        )
    }
}

fn gpt5() -> CatalogueEntry {
    entry("openai-api/gpt-5", "Builtin GPT", AuthIdentity::ApiKey)
}

#[test]
fn compose_publishes_runtime_and_catalogue_as_one_generation() {
    let fixture = Fixture::new(vec![gpt5()]);
    let composed = fixture.compose().expect("composition succeeds");
    assert_eq!(composed.snapshot.generation(), 1);
    assert_eq!(fixture.catalogue_store.current().generation(), 1);
    let published = fixture.runtime_store.current().expect("runtime published");
    assert_eq!(published.generation(), 1);
    assert_eq!(published.provider.name(), "router");
}

#[test]
fn runtime_store_is_empty_before_first_composition() {
    let store = RuntimeSnapshotStore::new();
    assert!(store.current().is_none());
}

#[test]
fn second_composition_publishes_next_generation_for_both_halves() {
    let fixture = Fixture::new(vec![gpt5()]);
    fixture.compose().expect("first composition succeeds");
    fixture.compose().expect("second composition succeeds");
    let published = fixture.runtime_store.current().expect("runtime published");
    // Atomicity: catalogue queries and routing must report the same
    // generation — never new catalogue + stale routing.
    assert_eq!(published.generation(), 2);
    assert_eq!(fixture.catalogue_store.current().generation(), 2);
}

#[test]
fn failed_composition_reports_error_and_retains_previous_runtime() {
    let fixture = Fixture::new(vec![gpt5()]);
    fixture.compose().expect("first composition succeeds");
    fixture.factory.fail_with("boom");
    let error = fixture.compose().expect_err("second composition fails");
    assert!(
        error.error.contains("boom"),
        "error carries cause: {error:?}"
    );
    let retained = error.retained.expect("previous runtime retained");
    assert_eq!(retained.generation(), 1);
    let published = fixture.runtime_store.current().expect("still published");
    assert_eq!(published.generation(), 1);
    assert_eq!(published.provider.name(), "router");
    // The catalogue store also stays on the last valid generation: a failed
    // composition must not advance one half and not the other.
    assert_eq!(fixture.catalogue_store.current().generation(), 1);
}

#[test]
fn failed_first_composition_publishes_nothing() {
    let fixture = Fixture::new(vec![gpt5()]);
    fixture.factory.fail_with("no credentials at all");
    let error = fixture.compose().expect_err("composition fails");
    assert!(error.retained.is_none());
    assert!(fixture.runtime_store.current().is_none());
    assert_eq!(fixture.catalogue_store.current().generation(), 0);
}

#[test]
fn selection_resolves_runnable_model_to_catalogue_identity() {
    let fixture = Fixture::new(vec![gpt5()]);
    fixture.compose().expect("composition succeeds");
    let reference = ModelRef::parse_qualified("openai-api/gpt-5").unwrap();
    let selection = ResolveModelSelectionUseCase::new()
        .select(&fixture.runtime_store, &reference)
        .expect("selection succeeds");
    // The selection surfaces exactly what the catalogue shows for this model:
    // provider identity, transport, auth path, and metadata.
    assert_eq!(selection.entry.provider.id.as_str(), "openai-api");
    assert_eq!(
        selection.entry.provider.transport,
        TransportKind::OpenAiCompletions
    );
    assert_eq!(selection.entry.provider.auth, AuthIdentity::ApiKey);
    assert_eq!(
        selection.entry.model.display_name.as_deref(),
        Some("Builtin GPT")
    );
    assert_eq!(selection.provider.name(), "router");
    assert_eq!(
        selection.generation,
        fixture.catalogue_store.current().generation()
    );
}

#[test]
fn selection_of_unknown_model_returns_structured_reason() {
    let fixture = Fixture::new(vec![gpt5()]);
    fixture.compose().expect("composition succeeds");
    let reference = ModelRef::parse_qualified("openai-api/no-such-model").unwrap();
    let error = ResolveModelSelectionUseCase::new()
        .select(&fixture.runtime_store, &reference)
        .expect_err("selection fails");
    assert_eq!(
        error,
        SelectionError::UnknownModel {
            reference: "openai-api/no-such-model".to_string()
        }
    );
}

#[test]
fn selection_without_credential_returns_missing_credential_reason() {
    let mut fixture = Fixture::new(vec![gpt5()]);
    fixture.credentials.denied.push("openai-api".to_string());
    fixture.compose().expect("composition succeeds");
    let reference = ModelRef::parse_qualified("openai-api/gpt-5").unwrap();
    let error = ResolveModelSelectionUseCase::new()
        .select(&fixture.runtime_store, &reference)
        .expect_err("selection fails");
    match error {
        SelectionError::NotRunnable { reasons, .. } => {
            assert!(
                reasons.contains(&UnavailableReason::MissingCredential),
                "structured missing-credential reason expected, got {reasons:?}"
            );
        }
        other => panic!("expected NotRunnable, got {other:?}"),
    }
}

#[test]
fn selection_before_any_composition_returns_no_runtime() {
    let store = RuntimeSnapshotStore::new();
    let reference = ModelRef::parse_qualified("openai-api/gpt-5").unwrap();
    let error = ResolveModelSelectionUseCase::new()
        .select(&store, &reference)
        .expect_err("selection fails");
    assert_eq!(error, SelectionError::NoRuntime);
}

#[test]
fn selection_never_swaps_api_key_and_oauth_identities() {
    let api = entry("openai-api/gpt-5", "API GPT", AuthIdentity::ApiKey);
    let oauth_provider = crate::domain::catalogue::ProviderId::new("openai").unwrap();
    let oauth = entry(
        "openai/gpt-5",
        "OAuth GPT",
        AuthIdentity::OAuth {
            provider: Some(oauth_provider),
        },
    );
    let fixture = Fixture::new(vec![api, oauth]);
    fixture.compose().expect("composition succeeds");
    let use_case = ResolveModelSelectionUseCase::new();

    let oauth_ref = ModelRef::parse_qualified("openai/gpt-5").unwrap();
    let selection = use_case
        .select(&fixture.runtime_store, &oauth_ref)
        .expect("oauth selection succeeds");
    assert!(
        matches!(selection.entry.provider.auth, AuthIdentity::OAuth { .. }),
        "OAuth model must keep its OAuth identity, got {:?}",
        selection.entry.provider.auth
    );

    let api_ref = ModelRef::parse_qualified("openai-api/gpt-5").unwrap();
    let selection = use_case
        .select(&fixture.runtime_store, &api_ref)
        .expect("api-key selection succeeds");
    assert_eq!(
        selection.entry.provider.auth,
        AuthIdentity::ApiKey,
        "API-key model must keep its API-key identity"
    );
}
