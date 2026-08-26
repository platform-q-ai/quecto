//! Steps for catalogue_runtime.feature (issue #1573, epic #1193 slice 3).
//!
//! Exercises the application runtime-composition and model-selection use
//! cases against fake ports only: compose/publish atomicity, failure
//! retention, selection identity, and structured unrunnable reasons.

use std::sync::{Arc, Mutex};

use super::*;
use quecto::application::catalogue::{
    CatalogueSnapshotStore, CatalogueSource, CredentialStatusPort, SourceEntries,
};
use quecto::application::provider_runtime::{
    ComposeProviderRuntimeUseCase, ComposedRuntime, CompositionPorts, ProviderRuntimeFactory,
    ResolveModelSelectionUseCase, RuntimeCompositionError, RuntimeSnapshotStore, SelectionError,
};
use quecto::domain::catalogue::{
    AuthIdentity, Availability, CatalogueEntry, ModelCapabilities, ModelCost, ModelDescriptor,
    ModelRef, ProviderDescriptor, ProviderId, SourceLayer, TransportKind, UnavailableReason,
};

#[derive(Debug, Default)]
pub struct CatalogueRuntimeState {
    sources: Vec<Arc<RtFakeSource>>,
    credential_denied: Vec<String>,
    factory: Option<Arc<RtFakeFactory>>,
    catalogue_store: Option<CatalogueSnapshotStore>,
    runtime_store: Option<RuntimeSnapshotStore>,
    composed: Option<Result<ComposedRuntime, RuntimeCompositionError>>,
    selection:
        Option<Result<quecto::application::provider_runtime::ModelSelection, SelectionError>>,
}

#[derive(Debug)]
struct RtFakeSource {
    id: String,
    layer: SourceLayer,
    entries: Mutex<Vec<CatalogueEntry>>,
}

impl CatalogueSource for RtFakeSource {
    fn id(&self) -> &str {
        &self.id
    }
    fn layer(&self) -> SourceLayer {
        self.layer
    }
    fn load(&self) -> Result<SourceEntries, String> {
        Ok(SourceEntries::from(self.entries.lock().unwrap().clone()))
    }
}

struct RtFakeCredentials {
    denied: Vec<String>,
}

impl CredentialStatusPort for RtFakeCredentials {
    fn credential_available(&self, entry: &CatalogueEntry) -> bool {
        !self.denied.iter().any(|p| p == entry.provider.id.as_str())
    }
}

#[derive(Debug)]
struct RtNamedProvider {
    name: String,
}

impl LlmProvider for RtNamedProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        Box::pin(async { Err(DomainError::Provider("bdd runtime fake".into())) })
    }
}

#[derive(Debug)]
struct RtFakeFactory {
    result: Mutex<Result<String, String>>,
}

impl ProviderRuntimeFactory<(), ()> for RtFakeFactory {
    fn compose_runtime(&self, _config: &(), _inputs: &()) -> Result<Arc<dyn LlmProvider>, String> {
        self.result
            .lock()
            .unwrap()
            .clone()
            .map(|name| Arc::new(RtNamedProvider { name }) as Arc<dyn LlmProvider>)
    }
}

fn rt_layer(name: &str) -> SourceLayer {
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

fn rt_transport(name: &str) -> TransportKind {
    match name {
        "openai-completions" => TransportKind::OpenAiCompletions,
        "anthropic-messages" => TransportKind::AnthropicMessages,
        "google-generative-ai" => TransportKind::GoogleGenerativeAi,
        other => panic!("unknown transport '{other}'"),
    }
}

fn rt_auth(name: &str, provider: &ProviderId) -> AuthIdentity {
    match name {
        "api-key" => AuthIdentity::ApiKey,
        "oauth" => AuthIdentity::OAuth {
            provider: Some(provider.clone()),
        },
        other => panic!("unknown auth '{other}'"),
    }
}

fn rt_capabilities() -> ModelCapabilities {
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

fn rt_entry(qualified: &str, display: &str, auth: &str) -> CatalogueEntry {
    let reference = ModelRef::parse_qualified(qualified).unwrap();
    let auth = rt_auth(auth, reference.provider());
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
            capabilities: rt_capabilities(),
            availability: Availability::runnable(),
        },
    }
}

fn rt_compose(world: &mut QuectoWorld) {
    let state = &mut world.catalogue_runtime;
    let catalogue_store = state
        .catalogue_store
        .get_or_insert_with(CatalogueSnapshotStore::empty)
        .clone();
    let runtime_store = state
        .runtime_store
        .get_or_insert_with(RuntimeSnapshotStore::new)
        .clone();
    let factory = state.factory.clone().expect("factory declared by a Given");
    let credentials = RtFakeCredentials {
        denied: state.credential_denied.clone(),
    };
    let sources = state.sources.clone();
    let refs: Vec<&dyn CatalogueSource> = sources
        .iter()
        .map(|source| source.as_ref() as &dyn CatalogueSource)
        .collect();
    state.composed = Some(ComposeProviderRuntimeUseCase::new().compose_and_publish(
        factory.as_ref(),
        &(),
        &(),
        &CompositionPorts {
            sources: &refs,
            credentials: &credentials,
            catalogue_store: &catalogue_store,
            runtime_store: &runtime_store,
        },
    ));
}

fn rt_published(
    world: &QuectoWorld,
) -> Arc<quecto::application::provider_runtime::CatalogueRuntimeSnapshot> {
    world
        .catalogue_runtime
        .runtime_store
        .as_ref()
        .expect("runtime store exists")
        .current()
        .expect("a runtime is published")
}

#[given(
    expr = "a runtime catalogue source {string} at layer {string} defining model {string} named {string}"
)]
fn given_runtime_source(
    world: &mut QuectoWorld,
    id: String,
    layer: String,
    qualified: String,
    display: String,
) {
    world.catalogue_runtime.sources.push(Arc::new(RtFakeSource {
        id,
        layer: rt_layer(&layer),
        entries: Mutex::new(vec![rt_entry(&qualified, &display, "api-key")]),
    }));
}

#[given(
    expr = "a runtime catalogue source {string} at layer {string} defining model {string} named {string} with auth {string}"
)]
fn given_runtime_source_with_auth(
    world: &mut QuectoWorld,
    id: String,
    layer: String,
    qualified: String,
    display: String,
    auth: String,
) {
    world.catalogue_runtime.sources.push(Arc::new(RtFakeSource {
        id,
        layer: rt_layer(&layer),
        entries: Mutex::new(vec![rt_entry(&qualified, &display, &auth)]),
    }));
}

#[given(expr = "a provider factory that composes a runtime named {string}")]
fn given_provider_factory(world: &mut QuectoWorld, name: String) {
    world.catalogue_runtime.factory = Some(Arc::new(RtFakeFactory {
        result: Mutex::new(Ok(name)),
    }));
}

#[given(expr = "the provider factory now fails with {string}")]
fn given_factory_fails(world: &mut QuectoWorld, error: String) {
    let factory = world
        .catalogue_runtime
        .factory
        .as_ref()
        .expect("factory declared by a Given");
    *factory.result.lock().unwrap() = Err(error);
}

#[given(expr = "no runtime credential is available for provider {string}")]
fn given_no_runtime_credential(world: &mut QuectoWorld, provider: String) {
    world.catalogue_runtime.credential_denied.push(provider);
}

#[given(expr = "the provider runtime has been composed and published")]
fn given_runtime_composed(world: &mut QuectoWorld) {
    rt_compose(world);
}

#[when(expr = "the provider runtime is composed and published")]
fn when_runtime_composed(world: &mut QuectoWorld) {
    rt_compose(world);
}

#[when(expr = "model {string} is selected")]
fn when_model_selected(world: &mut QuectoWorld, qualified: String) {
    let store = world
        .catalogue_runtime
        .runtime_store
        .as_ref()
        .expect("runtime store exists")
        .clone();
    let reference = ModelRef::parse_qualified(&qualified).unwrap();
    world.catalogue_runtime.selection =
        Some(ResolveModelSelectionUseCase::new().select(&store, &reference));
}

#[then(expr = "the published runtime generation matches the published catalogue generation")]
fn then_generations_match(world: &mut QuectoWorld) {
    let published = rt_published(world);
    let catalogue = world
        .catalogue_runtime
        .catalogue_store
        .as_ref()
        .expect("catalogue store exists")
        .current();
    assert_eq!(published.generation(), catalogue.generation());
}

#[then(expr = "the published runtime provider is named {string}")]
fn then_runtime_provider_named(world: &mut QuectoWorld, name: String) {
    assert_eq!(rt_published(world).provider.name(), name);
}

#[then(expr = "the published runtime generation is {int}")]
fn then_runtime_generation(world: &mut QuectoWorld, generation: u64) {
    assert_eq!(rt_published(world).generation(), generation);
}

#[then(expr = "the composition reports a runtime error containing {string}")]
fn then_composition_error(world: &mut QuectoWorld, fragment: String) {
    let composed = world
        .catalogue_runtime
        .composed
        .as_ref()
        .expect("a composition ran");
    let error = composed.as_ref().expect_err("composition failed");
    assert!(
        error.error.contains(&fragment),
        "expected error containing '{fragment}', got '{}'",
        error.error
    );
}

#[then(expr = "the selection succeeds with provider {string} transport {string} auth {string}")]
fn then_selection_identity(
    world: &mut QuectoWorld,
    provider: String,
    transport: String,
    auth: String,
) {
    let selection = world
        .catalogue_runtime
        .selection
        .as_ref()
        .expect("a selection ran")
        .as_ref()
        .expect("selection succeeded");
    assert_eq!(selection.entry.provider.id.as_str(), provider);
    assert_eq!(selection.entry.provider.transport, rt_transport(&transport));
    let expected_auth = rt_auth(&auth, &selection.entry.provider.id);
    assert_eq!(selection.entry.provider.auth, expected_auth);
}

#[then(expr = "the selection generation matches the published catalogue generation")]
fn then_selection_generation_matches(world: &mut QuectoWorld) {
    let generation = world
        .catalogue_runtime
        .selection
        .as_ref()
        .expect("a selection ran")
        .as_ref()
        .expect("selection succeeded")
        .generation;
    let catalogue = world
        .catalogue_runtime
        .catalogue_store
        .as_ref()
        .expect("catalogue store exists")
        .current();
    assert_eq!(generation, catalogue.generation());
}

#[then(expr = "the selection fails because the model is unknown")]
fn then_selection_unknown(world: &mut QuectoWorld) {
    let error = world
        .catalogue_runtime
        .selection
        .as_ref()
        .expect("a selection ran")
        .as_ref()
        .expect_err("selection failed");
    assert!(
        matches!(error, SelectionError::UnknownModel { .. }),
        "expected UnknownModel, got {error:?}"
    );
}

#[then(expr = "the selection fails because a credential is missing")]
fn then_selection_missing_credential(world: &mut QuectoWorld) {
    let error = world
        .catalogue_runtime
        .selection
        .as_ref()
        .expect("a selection ran")
        .as_ref()
        .expect_err("selection failed");
    match error {
        SelectionError::NotRunnable { reasons, .. } => assert!(
            reasons.contains(&UnavailableReason::MissingCredential),
            "expected MissingCredential, got {reasons:?}"
        ),
        other => panic!("expected NotRunnable, got {other:?}"),
    }
}
