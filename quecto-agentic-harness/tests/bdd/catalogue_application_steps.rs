//! Steps for catalogue_application.feature (issue #1572, epic #1193 slice 2).
//!
//! Exercises the application catalogue use cases against fake ports only:
//! resolve/publish, snapshot store retention, credential-derived availability,
//! snapshot-only queries, and the shared listing projection.

use std::sync::{Arc, Mutex};

use super::*;
use quecto::application::catalogue::{
    CatalogueQuery, CatalogueSnapshotStore, CatalogueSource, CredentialStatusPort,
    QueryCatalogueUseCase, ResolveCatalogueUseCase, ResolvedCatalogue, project_model_listing,
};
use quecto::domain::catalogue::{
    AuthIdentity, Availability, CatalogueEntry, ModelCapabilities, ModelCost, ModelDescriptor,
    ModelRef, ProviderDescriptor, ProviderId, SourceLayer, TransportKind, UnavailableReason,
};

const APP_FAKE_SECRET: &str = "sk-secret-123";

#[derive(Debug, Default)]
pub struct CatalogueApplicationState {
    sources: Vec<Arc<AppFakeSource>>,
    credential_denied: Vec<String>,
    store: Option<CatalogueSnapshotStore>,
    resolved: Option<ResolvedCatalogue>,
}

#[derive(Debug)]
pub struct AppFakeSource {
    id: String,
    layer: SourceLayer,
    result: Mutex<Result<Vec<CatalogueEntry>, String>>,
}

impl CatalogueSource for AppFakeSource {
    fn id(&self) -> &str {
        &self.id
    }
    fn layer(&self) -> SourceLayer {
        self.layer
    }
    fn load(&self) -> Result<Vec<CatalogueEntry>, String> {
        self.result.lock().unwrap().clone()
    }
}

struct AppFakeCredentials {
    denied: Vec<String>,
    secret: String,
}

impl CredentialStatusPort for AppFakeCredentials {
    fn credential_available(&self, provider: &ProviderDescriptor) -> bool {
        // The secret stays inside the port: only a boolean ever leaves.
        debug_assert!(!self.secret.is_empty());
        !self.denied.iter().any(|p| p == provider.id.as_str())
    }
}

fn app_capabilities() -> ModelCapabilities {
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

fn app_entry(qualified: &str, display: &str) -> CatalogueEntry {
    let reference = ModelRef::parse_qualified(qualified).unwrap();
    CatalogueEntry {
        provider: ProviderDescriptor {
            id: reference.provider().clone(),
            display_name: Some(reference.provider().as_str().to_string()),
            transport: TransportKind::OpenAiCompletions,
            auth: AuthIdentity::ApiKey,
        },
        model: ModelDescriptor {
            reference,
            display_name: Some(display.to_string()),
            capabilities: app_capabilities(),
            availability: Availability::runnable(),
        },
    }
}

fn app_layer(name: &str) -> SourceLayer {
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

fn app_store(world: &mut QuectoWorld) -> CatalogueSnapshotStore {
    world
        .catalogue_application
        .store
        .get_or_insert_with(CatalogueSnapshotStore::empty)
        .clone()
}

#[given(
    expr = "a catalogue source {string} at layer {string} defining model {string} named {string}"
)]
fn given_catalogue_source(
    world: &mut QuectoWorld,
    id: String,
    layer: String,
    qualified: String,
    display: String,
) {
    world
        .catalogue_application
        .sources
        .push(Arc::new(AppFakeSource {
            id,
            layer: app_layer(&layer),
            result: Mutex::new(Ok(vec![app_entry(&qualified, &display)])),
        }));
}

#[given(expr = "a malformed catalogue source {string} at layer {string} failing with {string}")]
fn given_malformed_source(world: &mut QuectoWorld, id: String, layer: String, error: String) {
    world
        .catalogue_application
        .sources
        .push(Arc::new(AppFakeSource {
            id,
            layer: app_layer(&layer),
            result: Mutex::new(Err(error)),
        }));
}

#[given(expr = "no credential is available for provider {string}")]
fn given_no_credential(world: &mut QuectoWorld, provider: String) {
    world.catalogue_application.credential_denied.push(provider);
}

#[when(expr = "the resolve-effective-catalogue use case runs")]
fn when_resolve_runs(world: &mut QuectoWorld) {
    let store = app_store(world);
    let credentials = AppFakeCredentials {
        denied: world.catalogue_application.credential_denied.clone(),
        secret: APP_FAKE_SECRET.to_string(),
    };
    let sources = world.catalogue_application.sources.clone();
    let refs: Vec<&dyn CatalogueSource> = sources
        .iter()
        .map(|source| source.as_ref() as &dyn CatalogueSource)
        .collect();
    world.catalogue_application.resolved =
        Some(ResolveCatalogueUseCase.resolve_and_publish(&refs, &credentials, &store));
}

#[when(expr = "the source {string} becomes malformed failing with {string}")]
fn when_source_becomes_malformed(world: &mut QuectoWorld, id: String, error: String) {
    let source = world
        .catalogue_application
        .sources
        .iter()
        .find(|source| source.id == id)
        .expect("unknown source");
    *source.result.lock().unwrap() = Err(error);
}

#[when(expr = "the source {string} additionally defines model {string} named {string}")]
fn when_source_grows(world: &mut QuectoWorld, id: String, qualified: String, display: String) {
    let source = world
        .catalogue_application
        .sources
        .iter()
        .find(|source| source.id == id)
        .expect("unknown source");
    let mut result = source.result.lock().unwrap();
    let mut entries = result.clone().expect("source not loadable");
    entries.push(app_entry(&qualified, &display));
    *result = Ok(entries);
}

#[then(expr = "the published snapshot has {int} model")]
fn then_published_model_count(world: &mut QuectoWorld, count: usize) {
    let store = app_store(world);
    assert_eq!(store.current().entries().len(), count);
}

#[then(expr = "the published model {string} is named {string}")]
fn then_published_model_named(world: &mut QuectoWorld, qualified: String, display: String) {
    let store = app_store(world);
    let snapshot = store.current();
    let entry = snapshot
        .find(&ModelRef::parse_qualified(&qualified).unwrap())
        .expect("model not in published snapshot");
    assert_eq!(entry.model.display_name.as_deref(), Some(display.as_str()));
}

#[then(expr = "the published snapshot generation is {int}")]
fn then_published_generation(world: &mut QuectoWorld, generation: u64) {
    let store = app_store(world);
    assert_eq!(store.current().generation(), generation);
}

#[then(expr = "the resolution reports a source error for {string} containing {string}")]
fn then_source_error_reported(world: &mut QuectoWorld, source: String, message: String) {
    let resolved = world
        .catalogue_application
        .resolved
        .as_ref()
        .expect("resolve use case did not run");
    assert!(
        resolved
            .source_errors
            .iter()
            .any(|error| error.source == source && error.error.contains(&message)),
        "no structured error for source '{source}' containing '{message}': {:?}",
        resolved.source_errors
    );
}

#[then(expr = "the published model {string} is not runnable because a credential is missing")]
fn then_model_missing_credential(world: &mut QuectoWorld, qualified: String) {
    let store = app_store(world);
    let snapshot = store.current();
    let entry = snapshot
        .find(&ModelRef::parse_qualified(&qualified).unwrap())
        .expect("model not in published snapshot");
    assert!(!entry.model.availability.is_runnable());
    assert!(
        entry
            .model
            .availability
            .reasons()
            .contains(&UnavailableReason::MissingCredential)
    );
}

#[then(expr = "the published snapshot never contains the credential value {string}")]
fn then_no_secret_in_snapshot(world: &mut QuectoWorld, secret: String) {
    let store = app_store(world);
    let snapshot = store.current();
    assert!(
        !snapshot.entries().is_empty(),
        "snapshot unexpectedly empty"
    );
    let rendered = format!("{snapshot:?}");
    assert!(
        !rendered.contains(&secret),
        "credential value leaked into the published snapshot"
    );
}

#[then(expr = "the query use case still lists {int} model for filter {string}")]
fn then_query_lists(world: &mut QuectoWorld, count: usize, filter: String) {
    let store = app_store(world);
    let query = QueryCatalogueUseCase::new(store);
    let filter = match filter.as_str() {
        "all" => CatalogueQuery::All,
        "known" => CatalogueQuery::Known,
        "available" => CatalogueQuery::Available,
        "runnable" => CatalogueQuery::Runnable,
        other => panic!("unknown filter '{other}'"),
    };
    assert_eq!(query.query(filter).entries().len(), count);
}

#[then(expr = "the shared model listing projection lists model {string} at generation {int}")]
fn then_projection_lists(world: &mut QuectoWorld, qualified: String, generation: u64) {
    let store = app_store(world);
    let listing = project_model_listing(&store.current());
    assert_eq!(listing.generation, generation);
    assert!(
        listing.rows.iter().any(|row| row.qualified_id == qualified),
        "projection missing '{qualified}': {:?}",
        listing.rows
    );
}
