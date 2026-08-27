//! Steps for catalogue_refresh.feature (issue #1574, epic #1193 slice 4).
//!
//! Exercises the application refresh use case against fake refreshable
//! sources only: per-source outcomes, partial-failure isolation,
//! unsupported-provider reporting, cancellation, bounds, precedence, and
//! secret redaction.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use quecto::application::catalogue::{
    CatalogueSnapshotStore, CatalogueSource, CredentialStatusPort, ResolveCatalogueUseCase,
    SourceEntries,
};
use quecto::application::catalogue_refresh::{
    CatalogueRefreshReport, NoopRedaction, RefreshBounds, RefreshCatalogueSourcesUseCase,
    RefreshChange, RefreshContext, RefreshError, RefreshPorts, RefreshRedactionPort,
    RefreshSelection, RefreshableCatalogueSource, SourceRefreshStatus,
};
use quecto::domain::catalogue::{
    AuthIdentity, Availability, CatalogueEntry, ModelCapabilities, ModelCost, ModelDescriptor,
    ModelRef, ProviderDescriptor, SourceLayer, TransportKind,
};

#[derive(Debug, Default)]
pub struct CatalogueRefreshState {
    refreshables: Vec<Arc<RefFakeSource>>,
    static_sources: Vec<Arc<RefStaticSource>>,
    store: Option<CatalogueSnapshotStore>,
    secret: Option<String>,
    bounds: Option<RefreshBounds>,
    report: Option<CatalogueRefreshReport>,
    generation_before: Option<u64>,
}

#[derive(Debug)]
enum RefBehaviour {
    Update(Vec<CatalogueEntry>),
    Unchanged,
    Unsupported(String),
    Fail(String),
    TriggerCancel,
    /// Sleep past the run's timeout before returning, so the run observes an
    /// over-budget refresh.
    OutliveTimeout,
}

#[derive(Debug)]
struct RefFakeSource {
    id: String,
    behaviour: RefBehaviour,
    cached: Mutex<Vec<CatalogueEntry>>,
    refresh_calls: AtomicUsize,
}

impl CatalogueSource for RefFakeSource {
    fn id(&self) -> &str {
        &self.id
    }
    fn layer(&self) -> SourceLayer {
        SourceLayer::Discovered
    }
    fn load(&self) -> Result<SourceEntries, String> {
        Ok(SourceEntries::from(self.cached.lock().unwrap().clone()))
    }
}

impl RefreshableCatalogueSource for RefFakeSource {
    fn refresh(&self, ctx: &RefreshContext) -> Result<RefreshChange, RefreshError> {
        self.refresh_calls.fetch_add(1, Ordering::SeqCst);
        match &self.behaviour {
            RefBehaviour::Update(entries) => {
                let models = entries.len();
                *self.cached.lock().unwrap() = entries.clone();
                Ok(RefreshChange::Updated { models })
            }
            RefBehaviour::Unchanged => Ok(RefreshChange::Unchanged {
                models: self.cached.lock().unwrap().len(),
            }),
            RefBehaviour::Unsupported(reason) => Err(RefreshError::Unsupported {
                reason: reason.clone(),
            }),
            RefBehaviour::Fail(reason) => Err(RefreshError::Failed {
                reason: reason.clone(),
            }),
            RefBehaviour::TriggerCancel => {
                ctx.cancel();
                Err(RefreshError::Cancelled)
            }
            RefBehaviour::OutliveTimeout => {
                std::thread::sleep(ctx.bounds.timeout + Duration::from_millis(50));
                Ok(RefreshChange::Unchanged { models: 0 })
            }
        }
    }
}

#[derive(Debug)]
struct RefStaticSource {
    id: String,
    layer: SourceLayer,
    entries: Vec<CatalogueEntry>,
}

impl CatalogueSource for RefStaticSource {
    fn id(&self) -> &str {
        &self.id
    }
    fn layer(&self) -> SourceLayer {
        self.layer
    }
    fn load(&self) -> Result<SourceEntries, String> {
        Ok(SourceEntries::from(self.entries.clone()))
    }
}

struct RefAllowAll;

impl CredentialStatusPort for RefAllowAll {
    fn credential_available(&self, _entry: &CatalogueEntry) -> bool {
        true
    }
}

struct RefSecretRedaction {
    secret: String,
}

impl RefreshRedactionPort for RefSecretRedaction {
    fn redact(&self, text: &str) -> String {
        text.replace(&self.secret, "[redacted]")
    }
}

fn ref_entry(qualified: &str, display: &str) -> CatalogueEntry {
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
            capabilities: ModelCapabilities {
                effort_levels: Vec::new(),
                input_modalities: vec!["text".to_string()],
                context_window: 128_000,
                max_output_tokens: 4096,
                context_window_explicit: true,
                max_output_tokens_explicit: false,
                reasoning: false,
                cost: ModelCost::default(),
            },
            availability: Availability::runnable(),
        },
    }
}

fn ref_store(world: &mut QuectoWorld) -> CatalogueSnapshotStore {
    world
        .catalogue_refresh
        .store
        .get_or_insert_with(CatalogueSnapshotStore::empty)
        .clone()
}

fn ref_source(world: &QuectoWorld, id: &str) -> Arc<RefFakeSource> {
    world
        .catalogue_refresh
        .refreshables
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("unknown refreshable source '{id}'"))
        .clone()
}

fn ref_outcome_status(world: &QuectoWorld, source: &str) -> SourceRefreshStatus {
    world
        .catalogue_refresh
        .report
        .as_ref()
        .expect("refresh has not run")
        .outcomes
        .iter()
        .find(|o| o.source == source)
        .unwrap_or_else(|| panic!("no outcome reported for source '{source}'"))
        .status
        .clone()
}

fn run_refresh(world: &mut QuectoWorld, selection: RefreshSelection, ctx: RefreshContext) {
    let store = ref_store(world);
    world.catalogue_refresh.generation_before = Some(store.current().generation());
    let refreshables = world.catalogue_refresh.refreshables.clone();
    let statics = world.catalogue_refresh.static_sources.clone();
    let refreshable_refs: Vec<&dyn RefreshableCatalogueSource> = refreshables
        .iter()
        .map(|s| s.as_ref() as &dyn RefreshableCatalogueSource)
        .collect();
    let mut sources: Vec<&dyn CatalogueSource> = refreshables
        .iter()
        .map(|s| s.as_ref() as &dyn CatalogueSource)
        .collect();
    sources.extend(statics.iter().map(|s| s.as_ref() as &dyn CatalogueSource));
    let redaction: Box<dyn RefreshRedactionPort> = match &world.catalogue_refresh.secret {
        Some(secret) => Box::new(RefSecretRedaction {
            secret: secret.clone(),
        }),
        None => Box::new(NoopRedaction),
    };
    let ports = RefreshPorts {
        refreshables: &refreshable_refs,
        sources: &sources,
        credentials: &RefAllowAll,
        store: &store,
        redaction: redaction.as_ref(),
    };
    world.catalogue_refresh.report =
        Some(RefreshCatalogueSourcesUseCase.refresh(&ports, &selection, &ctx));
}

#[given(
    expr = "a refreshable catalogue source {string} that will report models {string} and {string}"
)]
fn given_refreshable_updating(world: &mut QuectoWorld, id: String, first: String, second: String) {
    let entries = vec![
        ref_entry(&format!("{id}/{first}"), &first),
        ref_entry(&format!("{id}/{second}"), &second),
    ];
    world
        .catalogue_refresh
        .refreshables
        .push(Arc::new(RefFakeSource {
            id,
            behaviour: RefBehaviour::Update(entries),
            cached: Mutex::new(Vec::new()),
            refresh_calls: AtomicUsize::new(0),
        }));
}

#[given(expr = "a refreshable catalogue source {string} that will report no change")]
fn given_refreshable_unchanged(world: &mut QuectoWorld, id: String) {
    let cached = vec![ref_entry(&format!("{id}/cached"), "Cached")];
    world
        .catalogue_refresh
        .refreshables
        .push(Arc::new(RefFakeSource {
            id,
            behaviour: RefBehaviour::Unchanged,
            cached: Mutex::new(cached),
            refresh_calls: AtomicUsize::new(0),
        }));
}

#[given(expr = "a catalogue source {string} that does not support remote refresh because {string}")]
fn given_refreshable_unsupported(world: &mut QuectoWorld, id: String, reason: String) {
    world
        .catalogue_refresh
        .refreshables
        .push(Arc::new(RefFakeSource {
            id,
            behaviour: RefBehaviour::Unsupported(reason),
            cached: Mutex::new(Vec::new()),
            refresh_calls: AtomicUsize::new(0),
        }));
}

#[given(expr = "a refreshable catalogue source {string} that will fail with {string}")]
fn given_refreshable_failing(world: &mut QuectoWorld, id: String, reason: String) {
    world
        .catalogue_refresh
        .refreshables
        .push(Arc::new(RefFakeSource {
            id,
            behaviour: RefBehaviour::Fail(reason),
            cached: Mutex::new(Vec::new()),
            refresh_calls: AtomicUsize::new(0),
        }));
}

#[given(
    expr = "a refreshable catalogue source {string} that is still refreshing when the refresh is cancelled"
)]
fn given_refreshable_cancelling(world: &mut QuectoWorld, id: String) {
    world
        .catalogue_refresh
        .refreshables
        .push(Arc::new(RefFakeSource {
            id,
            behaviour: RefBehaviour::TriggerCancel,
            cached: Mutex::new(Vec::new()),
            refresh_calls: AtomicUsize::new(0),
        }));
}

#[given(expr = "a user-override catalogue source {string} naming model {string} {string}")]
fn given_user_override_source(
    world: &mut QuectoWorld,
    id: String,
    qualified: String,
    name: String,
) {
    world
        .catalogue_refresh
        .static_sources
        .push(Arc::new(RefStaticSource {
            id,
            layer: SourceLayer::UserOverride,
            entries: vec![ref_entry(&qualified, &name)],
        }));
}

#[given(expr = "a refresh timeout of {int} milliseconds")]
fn given_refresh_timeout(world: &mut QuectoWorld, millis: u64) {
    world.catalogue_refresh.bounds = Some(RefreshBounds {
        timeout: Duration::from_millis(millis),
        ..RefreshBounds::default()
    });
}

#[given(
    expr = "a refreshable catalogue source {string} whose refresh outlives the refresh timeout"
)]
fn given_refreshable_outliving_timeout(world: &mut QuectoWorld, id: String) {
    world
        .catalogue_refresh
        .refreshables
        .push(Arc::new(RefFakeSource {
            id,
            behaviour: RefBehaviour::OutliveTimeout,
            cached: Mutex::new(Vec::new()),
            refresh_calls: AtomicUsize::new(0),
        }));
}

#[given(
    expr = "a refreshable catalogue source {string} whose refresh fails with an error containing the credential secret {string}"
)]
fn given_refreshable_failing_with_secret(world: &mut QuectoWorld, id: String, secret: String) {
    // One Given establishes both the failing error text and the secret the
    // redactor knows, so the two can never drift apart and silently turn the
    // no-secret Then vacuous.
    world
        .catalogue_refresh
        .refreshables
        .push(Arc::new(RefFakeSource {
            id,
            behaviour: RefBehaviour::Fail(format!("401 unauthorized for bearer {secret}")),
            cached: Mutex::new(Vec::new()),
            refresh_calls: AtomicUsize::new(0),
        }));
    world.catalogue_refresh.secret = Some(secret);
}

#[given(expr = "the effective catalogue has been resolved and published for refresh")]
fn given_prepublished(world: &mut QuectoWorld) {
    let store = ref_store(world);
    let refreshables = world.catalogue_refresh.refreshables.clone();
    let statics = world.catalogue_refresh.static_sources.clone();
    let mut sources: Vec<&dyn CatalogueSource> = refreshables
        .iter()
        .map(|s| s.as_ref() as &dyn CatalogueSource)
        .collect();
    sources.extend(statics.iter().map(|s| s.as_ref() as &dyn CatalogueSource));
    ResolveCatalogueUseCase.resolve_and_publish(&sources, &RefAllowAll, &store);
}

#[when(expr = "all catalogue sources are refreshed")]
fn when_refresh_all(world: &mut QuectoWorld) {
    let bounds = world.catalogue_refresh.bounds.unwrap_or_default();
    run_refresh(world, RefreshSelection::All, RefreshContext::new(bounds));
}

#[when(expr = "only catalogue source {string} is refreshed")]
fn when_refresh_only(world: &mut QuectoWorld, id: String) {
    run_refresh(
        world,
        RefreshSelection::Only(vec![id]),
        RefreshContext::default(),
    );
}

#[then(expr = "the refresh outcome for {string} is updated with {int} models")]
fn then_outcome_updated(world: &mut QuectoWorld, source: String, models: usize) {
    assert_eq!(
        ref_outcome_status(world, &source),
        SourceRefreshStatus::Updated { models }
    );
}

#[then(expr = "the refresh outcome for {string} is unchanged")]
fn then_outcome_unchanged(world: &mut QuectoWorld, source: String) {
    assert!(
        matches!(
            ref_outcome_status(world, &source),
            SourceRefreshStatus::Unchanged { .. }
        ),
        "expected an unchanged outcome for {source}"
    );
}

#[then(expr = "the refresh outcome for {string} is unsupported mentioning {string}")]
fn then_outcome_unsupported(world: &mut QuectoWorld, source: String, fragment: String) {
    match ref_outcome_status(world, &source) {
        SourceRefreshStatus::Unsupported { reason } => {
            assert!(reason.contains(&fragment), "unsupported reason: {reason}");
        }
        other => panic!("expected unsupported outcome for '{source}', got {other:?}"),
    }
}

#[then(expr = "the refresh outcome for {string} is failed mentioning {string}")]
fn then_outcome_failed(world: &mut QuectoWorld, source: String, fragment: String) {
    match ref_outcome_status(world, &source) {
        SourceRefreshStatus::Failed { reason } => {
            assert!(reason.contains(&fragment), "failed reason: {reason}");
        }
        other => panic!("expected failed outcome for '{source}', got {other:?}"),
    }
}

#[then(expr = "the refresh outcome for {string} is cancelled")]
fn then_outcome_cancelled(world: &mut QuectoWorld, source: String) {
    assert_eq!(
        ref_outcome_status(world, &source),
        SourceRefreshStatus::Cancelled
    );
}

#[then(expr = "a new catalogue generation is published")]
fn then_new_generation_published(world: &mut QuectoWorld) {
    let before = world
        .catalogue_refresh
        .generation_before
        .expect("refresh has not run");
    let store = ref_store(world);
    assert!(
        store.current().generation() > before,
        "a successful refresh must publish a new catalogue generation"
    );
}

#[then(expr = "the published snapshot contains model {string}")]
fn then_snapshot_contains(world: &mut QuectoWorld, qualified: String) {
    let store = ref_store(world);
    let reference = ModelRef::parse_qualified(&qualified).unwrap();
    assert!(
        store.current().find(&reference).is_some(),
        "published snapshot is missing model '{qualified}'"
    );
}

#[then(expr = "the previously published catalogue generation is retained")]
fn then_previous_generation_retained(world: &mut QuectoWorld) {
    let before = world
        .catalogue_refresh
        .generation_before
        .expect("refresh has not run");
    let store = ref_store(world);
    assert_eq!(
        store.current().generation(),
        before,
        "a totally failed refresh must not publish a new generation"
    );
}

#[then(expr = "source {string} was never asked to refresh")]
fn then_never_refreshed(world: &mut QuectoWorld, id: String) {
    let source = ref_source(world, &id);
    assert_eq!(source.refresh_calls.load(Ordering::SeqCst), 0);
}

#[then(expr = "the refresh-published model {string} is named {string}")]
fn then_published_model_named(world: &mut QuectoWorld, qualified: String, name: String) {
    let store = ref_store(world);
    let reference = ModelRef::parse_qualified(&qualified).unwrap();
    let snapshot = store.current();
    let entry = snapshot
        .find(&reference)
        .unwrap_or_else(|| panic!("published snapshot is missing model '{qualified}'"));
    assert_eq!(entry.model.display_name.as_deref(), Some(name.as_str()));
}

#[then(expr = "no refresh outcome contains the secret {string}")]
fn then_no_secret_in_outcomes(world: &mut QuectoWorld, secret: String) {
    let report = world
        .catalogue_refresh
        .report
        .as_ref()
        .expect("refresh has not run");
    assert!(
        !report.outcomes.is_empty(),
        "refresh reported no outcomes at all"
    );
    for outcome in &report.outcomes {
        let text = format!("{outcome:?}");
        assert!(
            !text.contains(&secret),
            "refresh outcome leaked the secret: {text}"
        );
    }
}
