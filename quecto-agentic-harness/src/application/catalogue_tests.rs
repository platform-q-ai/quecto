//! RED tests for issue #1572 (epic #1193 slice 2): application resolve/query
//! use cases, snapshot store, and the shared listing projection, exercised
//! against fake ports only.

use std::sync::Mutex;

use super::*;
use crate::domain::catalogue::{
    AuthIdentity, AvailabilityStatus, ModelCapabilities, ModelCost, ModelDescriptor, ModelId,
    ModelRef, ProviderId, UnavailableReason,
};

const FAKE_SECRET: &str = "sk-secret-123";

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

fn provider(id: &str) -> ProviderDescriptor {
    ProviderDescriptor {
        id: ProviderId::new(id).unwrap(),
        display_name: Some(id.to_string()),
        transport: TransportKind::OpenAiCompletions,
        auth: AuthIdentity::ApiKey,
    }
}

fn entry(provider_id: &str, model: &str, display: &str) -> CatalogueEntry {
    CatalogueEntry {
        provider: provider(provider_id),
        model: ModelDescriptor {
            reference: ModelRef::new(
                ProviderId::new(provider_id).unwrap(),
                ModelId::new(model).unwrap(),
            ),
            display_name: Some(display.to_string()),
            capabilities: capabilities(),
            availability: Availability::runnable(),
        },
    }
}

/// Fake source port: entries or a load error, with a load counter so tests can
/// prove queries never re-read sources.
struct FakeSource {
    id: String,
    layer: SourceLayer,
    result: Mutex<Result<Vec<CatalogueEntry>, String>>,
    loads: Mutex<usize>,
}

impl FakeSource {
    fn ok(id: &str, layer: SourceLayer, entries: Vec<CatalogueEntry>) -> Self {
        Self {
            id: id.to_string(),
            layer,
            result: Mutex::new(Ok(entries)),
            loads: Mutex::new(0),
        }
    }

    fn failing(id: &str, layer: SourceLayer, error: &str) -> Self {
        Self {
            id: id.to_string(),
            layer,
            result: Mutex::new(Err(error.to_string())),
            loads: Mutex::new(0),
        }
    }

    fn set_result(&self, result: Result<Vec<CatalogueEntry>, String>) {
        *self.result.lock().unwrap() = result;
    }

    fn load_count(&self) -> usize {
        *self.loads.lock().unwrap()
    }
}

impl CatalogueSource for FakeSource {
    fn id(&self) -> &str {
        &self.id
    }
    fn layer(&self) -> SourceLayer {
        self.layer
    }
    fn load(&self) -> Result<Vec<CatalogueEntry>, String> {
        *self.loads.lock().unwrap() += 1;
        self.result.lock().unwrap().clone()
    }
}

/// Fake credential port that holds a secret value internally but only ever
/// answers yes/no, so tests can assert the secret never reaches a snapshot.
struct FakeCredentials {
    available_for: Vec<String>,
    secret: String,
}

impl FakeCredentials {
    fn granting(providers: &[&str]) -> Self {
        Self {
            available_for: providers.iter().map(|p| p.to_string()).collect(),
            secret: FAKE_SECRET.to_string(),
        }
    }
}

impl CredentialStatusPort for FakeCredentials {
    fn credential_available(&self, provider: &ProviderDescriptor) -> bool {
        // The secret stays inside the port: only a boolean ever leaves.
        debug_assert!(!self.secret.is_empty());
        self.available_for.iter().any(|p| p == provider.id.as_str())
    }
}

#[test]
fn resolve_applies_source_layer_precedence() {
    let builtin = FakeSource::ok(
        "builtin",
        SourceLayer::BuiltIn,
        vec![entry("openai-api", "gpt-5", "Builtin GPT")],
    );
    let user = FakeSource::ok(
        "user",
        SourceLayer::UserOverride,
        vec![entry("openai-api", "gpt-5", "My GPT")],
    );
    let store = CatalogueSnapshotStore::empty();
    let resolved = ResolveCatalogueUseCase.resolve_and_publish(
        &[&user, &builtin],
        &FakeCredentials::granting(&["openai-api"]),
        &store,
    );
    assert_eq!(resolved.snapshot.entries().len(), 1);
    assert_eq!(
        resolved.snapshot.entries()[0].model.display_name.as_deref(),
        Some("My GPT")
    );
}

#[test]
fn resolve_isolates_a_malformed_source_with_a_structured_error() {
    let builtin = FakeSource::ok(
        "builtin",
        SourceLayer::BuiltIn,
        vec![entry("openai-api", "gpt-5", "Builtin GPT")],
    );
    let broken = FakeSource::failing("broken", SourceLayer::UserDefined, "bad json");
    let store = CatalogueSnapshotStore::empty();
    let resolved = ResolveCatalogueUseCase.resolve_and_publish(
        &[&builtin, &broken],
        &FakeCredentials::granting(&["openai-api"]),
        &store,
    );
    assert_eq!(resolved.snapshot.entries().len(), 1);
    assert_eq!(
        resolved.source_errors,
        vec![CatalogueSourceError {
            source: "broken".to_string(),
            error: "bad json".to_string(),
        }]
    );
}

#[test]
fn resolve_publishes_atomically_with_a_new_generation() {
    let builtin = FakeSource::ok(
        "builtin",
        SourceLayer::BuiltIn,
        vec![entry("openai-api", "gpt-5", "Builtin GPT")],
    );
    let store = CatalogueSnapshotStore::empty();
    let resolved = ResolveCatalogueUseCase.resolve_and_publish(
        &[&builtin],
        &FakeCredentials::granting(&["openai-api"]),
        &store,
    );
    assert_eq!(resolved.snapshot.generation(), 1);
    // The store's current snapshot IS the resolved one — no partial state.
    assert_eq!(store.current(), resolved.snapshot);
}

#[test]
fn resolve_retains_last_valid_snapshot_when_every_source_fails() {
    let builtin = FakeSource::ok(
        "builtin",
        SourceLayer::BuiltIn,
        vec![entry("openai-api", "gpt-5", "Builtin GPT")],
    );
    let store = CatalogueSnapshotStore::empty();
    let credentials = FakeCredentials::granting(&["openai-api"]);
    ResolveCatalogueUseCase.resolve_and_publish(&[&builtin], &credentials, &store);
    let valid = store.current();
    assert_eq!(valid.entries().len(), 1);

    builtin.set_result(Err("disk error".to_string()));
    let resolved = ResolveCatalogueUseCase.resolve_and_publish(&[&builtin], &credentials, &store);
    assert_eq!(resolved.source_errors.len(), 1);
    assert_eq!(
        store.current(),
        valid,
        "last valid snapshot must be retained"
    );
    assert_eq!(store.current().generation(), 1);
}

#[test]
fn resolve_derives_availability_from_credential_status() {
    let builtin = FakeSource::ok(
        "builtin",
        SourceLayer::BuiltIn,
        vec![
            entry("openai-api", "gpt-5", "Builtin GPT"),
            entry("anthropic", "opus", "Opus"),
        ],
    );
    let store = CatalogueSnapshotStore::empty();
    // Credential only for anthropic — openai-api must come out non-runnable.
    ResolveCatalogueUseCase.resolve_and_publish(
        &[&builtin],
        &FakeCredentials::granting(&["anthropic"]),
        &store,
    );
    let snapshot = store.current();
    let openai = snapshot
        .find(&ModelRef::parse("openai-api", "gpt-5").unwrap())
        .unwrap();
    assert!(!openai.model.availability.is_runnable());
    assert!(
        openai
            .model
            .availability
            .reasons()
            .contains(&UnavailableReason::MissingCredential)
    );
    let anthropic = snapshot
        .find(&ModelRef::parse("anthropic", "opus").unwrap())
        .unwrap();
    assert!(anthropic.model.availability.is_runnable());
}

#[test]
fn derive_availability_composes_adapter_and_credential_status() {
    let runnable = derive_availability(TransportKind::OpenAiCompletions, true, true);
    assert!(runnable.is_runnable());

    let no_credential = derive_availability(TransportKind::OpenAiCompletions, true, false);
    assert_eq!(no_credential.status(), AvailabilityStatus::Available);
    assert_eq!(
        no_credential.reasons(),
        &[UnavailableReason::MissingCredential]
    );

    let no_adapter = derive_availability(TransportKind::OpenAiCompletions, false, true);
    assert!(!no_adapter.is_runnable());
    assert!(
        no_adapter
            .reasons()
            .iter()
            .any(|reason| matches!(reason, UnavailableReason::UnsupportedTransport { .. }))
    );
}

#[test]
fn secrets_never_appear_in_a_published_snapshot() {
    let builtin = FakeSource::ok(
        "builtin",
        SourceLayer::BuiltIn,
        vec![entry("openai-api", "gpt-5", "Builtin GPT")],
    );
    let store = CatalogueSnapshotStore::empty();
    ResolveCatalogueUseCase.resolve_and_publish(
        &[&builtin],
        &FakeCredentials::granting(&["openai-api"]),
        &store,
    );
    let rendered = format!("{:?}", store.current());
    assert!(
        !rendered.contains(FAKE_SECRET),
        "snapshot must never carry credential values"
    );
    assert_eq!(store.current().entries().len(), 1);
}

#[test]
fn query_reads_the_snapshot_only_and_never_reloads_sources() {
    let builtin = FakeSource::ok(
        "builtin",
        SourceLayer::BuiltIn,
        vec![entry("openai-api", "gpt-5", "Builtin GPT")],
    );
    let store = CatalogueSnapshotStore::empty();
    ResolveCatalogueUseCase.resolve_and_publish(
        &[&builtin],
        &FakeCredentials::granting(&["openai-api"]),
        &store,
    );
    let loads_after_resolve = builtin.load_count();
    // Source grows a new model AFTER publication; queries must not see it.
    builtin.set_result(Ok(vec![
        entry("openai-api", "gpt-5", "Builtin GPT"),
        entry("openai-api", "gpt-6", "Next GPT"),
    ]));
    let query = QueryCatalogueUseCase::new(store.clone());
    let all = query.query(CatalogueQuery::All);
    assert_eq!(all.entries().len(), 1);
    assert_eq!(all.generation(), 1);
    assert_eq!(
        builtin.load_count(),
        loads_after_resolve,
        "query must never re-read sources"
    );
}

#[test]
fn query_filters_narrow_by_availability() {
    let builtin = FakeSource::ok(
        "builtin",
        SourceLayer::BuiltIn,
        vec![
            entry("openai-api", "gpt-5", "Builtin GPT"),
            entry("anthropic", "opus", "Opus"),
        ],
    );
    let store = CatalogueSnapshotStore::empty();
    ResolveCatalogueUseCase.resolve_and_publish(
        &[&builtin],
        &FakeCredentials::granting(&["anthropic"]),
        &store,
    );
    let query = QueryCatalogueUseCase::new(store.clone());
    assert_eq!(query.query(CatalogueQuery::All).entries().len(), 2);
    assert_eq!(query.query(CatalogueQuery::Known).entries().len(), 2);
    let runnable = query.query(CatalogueQuery::Runnable);
    assert_eq!(runnable.entries().len(), 1);
    assert_eq!(
        runnable.entries()[0].reference().qualified_id(),
        "anthropic/opus"
    );
    // Filtered projections keep the source generation.
    assert_eq!(runnable.generation(), store.current().generation());
}

#[test]
fn all_consumer_surfaces_project_the_same_generation() {
    let builtin = FakeSource::ok(
        "builtin",
        SourceLayer::BuiltIn,
        vec![entry("openai-api", "gpt-5", "Builtin GPT")],
    );
    let store = CatalogueSnapshotStore::empty();
    ResolveCatalogueUseCase.resolve_and_publish(
        &[&builtin],
        &FakeCredentials::granting(&["openai-api"]),
        &store,
    );
    // CLI, UDS, and TUI all render this one projection over the store.
    let cli = project_model_listing(&store.current());
    let uds = project_model_listing(&store.current());
    let tui = project_model_listing(&store.current());
    assert_eq!(cli.generation, 1);
    assert_eq!(cli, uds);
    assert_eq!(uds, tui);
    assert_eq!(cli.rows.len(), 1);
    assert_eq!(cli.rows[0].qualified_id, "openai-api/gpt-5");
    assert_eq!(cli.rows[0].display_name.as_deref(), Some("Builtin GPT"));
    assert!(cli.rows[0].runnable);
}
