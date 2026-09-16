//! Unit tests for the catalogue refresh use case (epic #1193 slice 4,
//! #1846): the run policy over fake ports, and the loader seam.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;
use crate::application::catalogue::ports::NoopRedaction;
use crate::application::catalogue::{CatalogueSource, CredentialStatusPort, SourceEntries};

/// Inputs borrowed from a rig's stack fakes.
pub(super) struct BorrowedInputs<'a> {
    pub(super) refreshables: &'a [&'a dyn RefreshableCatalogueSource],
    pub(super) sources: &'a [&'a dyn CatalogueSource],
    pub(super) redaction: &'a dyn RefreshRedactionPort,
}

impl LoadedRefreshInputs for BorrowedInputs<'_> {
    fn refreshables(&self) -> Vec<&dyn RefreshableCatalogueSource> {
        self.refreshables.to_vec()
    }
    fn sources(&self) -> Vec<&dyn CatalogueSource> {
        self.sources.to_vec()
    }
    fn credentials(&self) -> &dyn CredentialStatusPort {
        &AllowAllCredentials
    }
    fn redaction(&self) -> &dyn RefreshRedactionPort {
        self.redaction
    }
}

/// A loader whose load fails, or that is never expected to load.
pub(super) struct FailingLoader(pub(super) String);

impl RefreshInputsLoader for FailingLoader {
    fn load(&self) -> Result<Box<dyn LoadedRefreshInputs>, String> {
        Err(self.0.clone())
    }
}

pub(super) fn use_case(store: &CatalogueSnapshotStore) -> RefreshCatalogueSources {
    RefreshCatalogueSources::new(
        Arc::new(FailingLoader("rig: the loader is not used".into())),
        store.clone(),
    )
}
use crate::domain::catalogue::{
    AuthIdentity, Availability, CatalogueEntry, ModelCapabilities, ModelCost, ModelDescriptor,
    ModelRef, ProviderDescriptor, SourceLayer, TransportKind,
};

pub(super) fn entry(qualified: &str, display: &str) -> CatalogueEntry {
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

/// What a fake source does when asked to refresh.
pub(super) enum Behaviour {
    /// Replace the cached entries with these and report `Updated`.
    Update(Vec<CatalogueEntry>),
    Unchanged,
    Unsupported(String),
    Fail(String),
    /// Trigger cooperative cancellation mid-run, then report `Cancelled`.
    TriggerCancel,
}

pub(super) struct FakeRefreshable {
    id: String,
    layer: SourceLayer,
    cached: Mutex<Vec<CatalogueEntry>>,
    behaviour: Behaviour,
    refresh_calls: AtomicUsize,
    observed_bounds: Mutex<Option<RefreshBounds>>,
}

impl FakeRefreshable {
    pub(super) fn new(id: &str, behaviour: Behaviour) -> Self {
        Self {
            id: id.to_string(),
            layer: SourceLayer::Discovered,
            cached: Mutex::new(Vec::new()),
            behaviour,
            refresh_calls: AtomicUsize::new(0),
            observed_bounds: Mutex::new(None),
        }
    }

    pub(super) fn with_cached(self, cached: Vec<CatalogueEntry>) -> Self {
        *self.cached.lock().unwrap() = cached;
        self
    }

    pub(super) fn calls(&self) -> usize {
        self.refresh_calls.load(Ordering::SeqCst)
    }
}

impl CatalogueSource for FakeRefreshable {
    fn id(&self) -> &str {
        &self.id
    }
    fn layer(&self) -> SourceLayer {
        self.layer
    }
    fn load(&self) -> Result<SourceEntries, String> {
        Ok(SourceEntries::from(self.cached.lock().unwrap().clone()))
    }
}

impl RefreshableCatalogueSource for FakeRefreshable {
    fn refresh(&self, ctx: &RefreshContext) -> Result<RefreshChange, RefreshError> {
        self.refresh_calls.fetch_add(1, Ordering::SeqCst);
        *self.observed_bounds.lock().unwrap() = Some(ctx.bounds);
        match &self.behaviour {
            Behaviour::Update(entries) => {
                let models = entries.len();
                *self.cached.lock().unwrap() = entries.clone();
                Ok(RefreshChange::Updated { models })
            }
            Behaviour::Unchanged => Ok(RefreshChange::Unchanged {
                models: self.cached.lock().unwrap().len(),
            }),
            Behaviour::Unsupported(reason) => Err(RefreshError::Unsupported {
                reason: reason.clone(),
            }),
            Behaviour::Fail(reason) => Err(RefreshError::Failed {
                reason: reason.clone(),
            }),
            Behaviour::TriggerCancel => {
                ctx.cancel();
                Err(RefreshError::Cancelled)
            }
        }
    }
}

/// A plain (non-refreshable) source for precedence layers.
pub(super) struct StaticSource {
    pub(super) id: String,
    pub(super) layer: SourceLayer,
    pub(super) entries: Vec<CatalogueEntry>,
}

impl CatalogueSource for StaticSource {
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

pub(super) struct AllowAllCredentials;

impl CredentialStatusPort for AllowAllCredentials {
    fn credential_available(&self, _entry: &CatalogueEntry) -> bool {
        true
    }
}

struct SecretRedaction {
    secret: String,
}

impl RefreshRedactionPort for SecretRedaction {
    fn redact(&self, text: &str) -> String {
        text.replace(&self.secret, "[redacted]")
    }
}

pub(super) fn outcome<'a>(
    report: &'a CatalogueRefreshReport,
    source: &str,
) -> &'a SourceRefreshOutcome {
    report
        .outcomes
        .iter()
        .find(|o| o.source == source)
        .unwrap_or_else(|| panic!("no outcome reported for source '{source}'"))
}

pub(super) fn run(
    refreshables: &[&FakeRefreshable],
    extra_sources: &[&dyn CatalogueSource],
    store: &CatalogueSnapshotStore,
    selection: &RefreshSelection,
    ctx: &RefreshContext,
    redaction: &dyn RefreshRedactionPort,
) -> CatalogueRefreshReport {
    let refreshable_refs: Vec<&dyn RefreshableCatalogueSource> = refreshables
        .iter()
        .map(|s| *s as &dyn RefreshableCatalogueSource)
        .collect();
    let mut sources: Vec<&dyn CatalogueSource> = refreshables
        .iter()
        .map(|s| *s as &dyn CatalogueSource)
        .collect();
    sources.extend_from_slice(extra_sources);
    let inputs = BorrowedInputs {
        refreshables: &refreshable_refs,
        sources: &sources,
        redaction,
    };
    use_case(store).refresh_loaded(&inputs, selection, ctx)
}

pub(super) fn prepublish(store: &CatalogueSnapshotStore, sources: &[&dyn CatalogueSource]) {
    ResolveCatalogueUseCase.resolve_and_publish(sources, &AllowAllCredentials, store);
}

#[test]
fn refresh_all_reports_outcomes_and_publishes_next_generation() {
    let openrouter = FakeRefreshable::new(
        "openrouter",
        Behaviour::Update(vec![
            entry("openrouter/alpha", "Alpha"),
            entry("openrouter/beta", "Beta"),
        ]),
    );
    let local = FakeRefreshable::new("local", Behaviour::Unchanged)
        .with_cached(vec![entry("local/tiny", "Tiny")]);
    let store = CatalogueSnapshotStore::empty();
    prepublish(&store, &[&local as &dyn CatalogueSource]);
    assert_eq!(store.current().generation(), 1);

    let report = run(
        &[&openrouter, &local],
        &[],
        &store,
        &RefreshSelection::All,
        &RefreshContext::default(),
        &NoopRedaction,
    );

    assert_eq!(
        outcome(&report, "openrouter").status,
        SourceRefreshStatus::Updated { models: 2 }
    );
    assert_eq!(
        outcome(&report, "local").status,
        SourceRefreshStatus::Unchanged { models: 1 }
    );
    assert!(
        report.resolved.is_some(),
        "refresh must republish via resolve"
    );
    let snapshot = store.current();
    assert_eq!(
        snapshot.generation(),
        2,
        "refresh publishes the next generation"
    );
    assert!(
        snapshot
            .find(&ModelRef::parse_qualified("openrouter/alpha").unwrap())
            .is_some(),
        "refreshed model must be in the published snapshot"
    );
}

#[test]
fn refresh_subset_touches_only_selected_sources() {
    let openrouter = FakeRefreshable::new(
        "openrouter",
        Behaviour::Update(vec![entry("openrouter/alpha", "Alpha")]),
    );
    let local = FakeRefreshable::new("local", Behaviour::Unchanged);
    let store = CatalogueSnapshotStore::empty();

    let report = run(
        &[&openrouter, &local],
        &[],
        &store,
        &RefreshSelection::Only(vec!["openrouter".to_string()]),
        &RefreshContext::default(),
        &NoopRedaction,
    );

    assert_eq!(
        outcome(&report, "openrouter").status,
        SourceRefreshStatus::Updated { models: 1 }
    );
    assert_eq!(
        local.calls(),
        0,
        "unselected source must not be asked to refresh"
    );
    assert!(
        report.outcomes.iter().all(|o| o.source != "local"),
        "unselected source must not appear in the outcomes"
    );
}

#[test]
fn unsupported_source_reports_actionable_reason() {
    let anthropic = FakeRefreshable::new(
        "anthropic",
        Behaviour::Unsupported("provider does not expose a model listing endpoint".to_string()),
    );
    let store = CatalogueSnapshotStore::empty();

    let report = run(
        &[&anthropic],
        &[],
        &store,
        &RefreshSelection::All,
        &RefreshContext::default(),
        &NoopRedaction,
    );

    match &outcome(&report, "anthropic").status {
        SourceRefreshStatus::Unsupported { reason } => {
            assert!(
                reason.contains("model listing endpoint"),
                "unsupported reason must be actionable, got: {reason}"
            );
        }
        other => panic!("expected unsupported outcome, got {other:?}"),
    }
}

#[test]
fn partial_failure_keeps_other_sources_successes() {
    let openrouter = FakeRefreshable::new(
        "openrouter",
        Behaviour::Update(vec![entry("openrouter/alpha", "Alpha")]),
    );
    let flaky = FakeRefreshable::new("flaky", Behaviour::Fail("connection refused".to_string()));
    let store = CatalogueSnapshotStore::empty();

    let report = run(
        &[&openrouter, &flaky],
        &[],
        &store,
        &RefreshSelection::All,
        &RefreshContext::default(),
        &NoopRedaction,
    );

    match &outcome(&report, "flaky").status {
        SourceRefreshStatus::Failed { reason } => {
            assert!(reason.contains("connection refused"), "got: {reason}");
        }
        other => panic!("expected failed outcome, got {other:?}"),
    }
    assert!(
        store
            .current()
            .find(&ModelRef::parse_qualified("openrouter/alpha").unwrap())
            .is_some(),
        "one source's failure must not discard another's success"
    );
}

#[test]
fn total_failure_retains_previous_valid_snapshot() {
    let seed = StaticSource {
        id: "seed".to_string(),
        layer: SourceLayer::BuiltIn,
        entries: vec![entry("openai-api/gpt-5", "GPT 5")],
    };
    let flaky = FakeRefreshable::new("flaky", Behaviour::Fail("connection refused".to_string()));
    let store = CatalogueSnapshotStore::empty();
    prepublish(&store, &[&seed as &dyn CatalogueSource]);
    let before = store.current();
    assert_eq!(before.generation(), 1);

    let report = run(
        &[&flaky],
        &[&seed],
        &store,
        &RefreshSelection::All,
        &RefreshContext::default(),
        &NoopRedaction,
    );

    match &outcome(&report, "flaky").status {
        SourceRefreshStatus::Failed { reason } => {
            assert!(reason.contains("connection refused"), "got: {reason}");
        }
        other => panic!("expected failed outcome, got {other:?}"),
    }
    let after = store.current();
    assert_eq!(
        after.generation(),
        before.generation(),
        "a totally failed refresh must not publish a new generation"
    );
    assert!(
        after
            .find(&ModelRef::parse_qualified("openai-api/gpt-5").unwrap())
            .is_some(),
        "the previous valid catalogue must be retained"
    );
}

#[test]
fn cancellation_preserves_completed_successes_and_skips_the_rest() {
    let openrouter = FakeRefreshable::new(
        "openrouter",
        Behaviour::Update(vec![entry("openrouter/alpha", "Alpha")]),
    );
    let slow = FakeRefreshable::new("slow", Behaviour::TriggerCancel);
    let later = FakeRefreshable::new("later", Behaviour::Unchanged);
    let store = CatalogueSnapshotStore::empty();

    let report = run(
        &[&openrouter, &slow, &later],
        &[],
        &store,
        &RefreshSelection::All,
        &RefreshContext::default(),
        &NoopRedaction,
    );

    assert_eq!(
        outcome(&report, "openrouter").status,
        SourceRefreshStatus::Updated { models: 1 }
    );
    assert_eq!(
        outcome(&report, "slow").status,
        SourceRefreshStatus::Cancelled
    );
    assert_eq!(
        outcome(&report, "later").status,
        SourceRefreshStatus::Cancelled
    );
    assert_eq!(
        later.calls(),
        0,
        "sources after cancellation must not be asked"
    );
    assert!(
        store
            .current()
            .find(&ModelRef::parse_qualified("openrouter/alpha").unwrap())
            .is_some(),
        "cancellation must not discard completed successes"
    );
}

#[test]
fn refresh_bounds_reach_the_sources() {
    let openrouter = FakeRefreshable::new(
        "openrouter",
        Behaviour::Update(vec![entry("openrouter/alpha", "Alpha")]),
    );
    let store = CatalogueSnapshotStore::empty();
    let bounds = RefreshBounds {
        timeout: Duration::from_secs(5),
        max_response_bytes: 1_048_576,
    };

    run(
        &[&openrouter],
        &[],
        &store,
        &RefreshSelection::All,
        &RefreshContext::new(bounds),
        &NoopRedaction,
    );

    assert_eq!(
        *openrouter.observed_bounds.lock().unwrap(),
        Some(bounds),
        "the configured bounds must reach the refreshing source"
    );
}

#[test]
fn user_override_still_wins_over_refreshed_discovered_data() {
    let openrouter = FakeRefreshable::new(
        "openrouter",
        Behaviour::Update(vec![entry("openrouter/alpha", "Alpha")]),
    );
    let user = StaticSource {
        id: "user".to_string(),
        layer: SourceLayer::UserOverride,
        entries: vec![entry("openrouter/alpha", "My Alpha")],
    };
    let store = CatalogueSnapshotStore::empty();

    run(
        &[&openrouter],
        &[&user],
        &store,
        &RefreshSelection::All,
        &RefreshContext::default(),
        &NoopRedaction,
    );

    let snapshot = store.current();
    let published = snapshot
        .find(&ModelRef::parse_qualified("openrouter/alpha").unwrap())
        .expect("refreshed model must be published");
    assert_eq!(
        published.model.display_name.as_deref(),
        Some("My Alpha"),
        "a user override must still win after a refresh"
    );
}

#[test]
fn failure_and_unsupported_reasons_are_redacted() {
    let secret = "sk-refresh-secret-123";
    let flaky = FakeRefreshable::new(
        "flaky",
        Behaviour::Fail(format!("401 unauthorized for bearer {secret}")),
    );
    let bare = FakeRefreshable::new(
        "bare",
        Behaviour::Unsupported(format!("no listing endpoint for token {secret}")),
    );
    let store = CatalogueSnapshotStore::empty();

    let report = run(
        &[&flaky, &bare],
        &[],
        &store,
        &RefreshSelection::All,
        &RefreshContext::default(),
        &SecretRedaction {
            secret: secret.to_string(),
        },
    );

    match &outcome(&report, "flaky").status {
        SourceRefreshStatus::Failed { reason } => {
            assert!(
                !reason.contains(secret),
                "outcome leaked the secret: {reason}"
            );
            assert!(reason.contains("[redacted]"), "got: {reason}");
            assert!(reason.contains("401 unauthorized"), "got: {reason}");
        }
        other => panic!("expected failed outcome, got {other:?}"),
    }
    match &outcome(&report, "bare").status {
        SourceRefreshStatus::Unsupported { reason } => {
            assert!(
                !reason.contains(secret),
                "unsupported outcome leaked the secret: {reason}"
            );
            assert!(reason.contains("[redacted]"), "got: {reason}");
        }
        other => panic!("expected unsupported outcome, got {other:?}"),
    }
}
