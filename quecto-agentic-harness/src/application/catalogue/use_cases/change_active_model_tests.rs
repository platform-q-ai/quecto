//! `ChangeActiveModel` against fake ports: the plan reads limits from the
//! generation it just published, carries the runtime's verdict, and the
//! switch applies model, limits and the effort reset together.

use std::sync::{Arc, Mutex};

use super::*;
use crate::application::catalogue::dto::EffortChangeRequest;
use crate::application::catalogue::ports::{
    DefaultScope, EffortRuntime, EffortVocabularySource, LoadedCatalogueInputs, RecordedDefaults,
    RuntimeSnapshotSource,
};
use crate::application::catalogue::{CatalogueSource, CredentialStatusPort, SourceEntries};
use crate::domain::catalogue::{
    AuthIdentity, Availability, CatalogueEntry, ModelCapabilities, ModelCost, ModelDescriptor,
    ModelId, ProviderDescriptor, ProviderId, SourceLayer, TransportKind, UnavailableReason,
};
use crate::domain::provider::EffortLevel;

fn entry(provider: &str, model: &str, explicit_limits: Option<(u32, u32)>) -> CatalogueEntry {
    let (max_output_tokens, context_window) = explicit_limits.unwrap_or((4096, 128_000));
    CatalogueEntry {
        provider: ProviderDescriptor {
            id: ProviderId::new(provider).unwrap(),
            display_name: None,
            transport: TransportKind::OpenAiCompletions,
            auth: AuthIdentity::ApiKey,
        },
        model: ModelDescriptor {
            reference: ModelRef::new(
                ProviderId::new(provider).unwrap(),
                ModelId::new(model).unwrap(),
            ),
            display_name: None,
            capabilities: ModelCapabilities {
                effort_levels: vec!["low".into(), "medium".into(), "high".into()],
                input_modalities: vec!["text".into()],
                context_window,
                max_output_tokens,
                context_window_explicit: explicit_limits.is_some(),
                max_output_tokens_explicit: explicit_limits.is_some(),
                reasoning: true,
                cost: ModelCost::default(),
            },
            availability: Availability::runnable(),
        },
    }
}

struct FakeSource(Mutex<Vec<CatalogueEntry>>);

impl CatalogueSource for FakeSource {
    fn id(&self) -> &str {
        "builtin"
    }
    fn layer(&self) -> SourceLayer {
        SourceLayer::BuiltIn
    }
    fn load(&self) -> Result<SourceEntries, String> {
        Ok(self.0.lock().unwrap().clone().into())
    }
}

struct GrantAll;

impl CredentialStatusPort for GrantAll {
    fn credential_available(&self, _entry: &CatalogueEntry) -> bool {
        true
    }
}

struct Loaded(Arc<FakeSource>);

impl LoadedCatalogueInputs for Loaded {
    fn sources(&self) -> Vec<&dyn CatalogueSource> {
        vec![self.0.as_ref()]
    }
    fn credentials(&self) -> &dyn CredentialStatusPort {
        &GrantAll
    }
}

struct Loader {
    source: Arc<FakeSource>,
    loads: Mutex<usize>,
}

impl CatalogueInputsLoader for Loader {
    fn load(&self) -> Box<dyn LoadedCatalogueInputs> {
        *self.loads.lock().unwrap() += 1;
        Box::new(Loaded(self.source.clone()))
    }
}

/// A silent provider standing in for the composed router.
#[derive(Debug)]
struct SilentProvider;

impl crate::application::providers::ports::LlmProvider for SilentProvider {
    fn name(&self) -> &str {
        "router"
    }
    fn chat<'a>(
        &'a self,
        _request: crate::application::providers::ports::ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        crate::domain::message::LlmResponse,
                        crate::domain::error::DomainError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async { Err(crate::domain::error::DomainError::Provider("silent".into())) })
    }
}

/// A runtime generation built over the given catalogue entries, or none.
struct FakeRuntime(Option<Arc<crate::application::provider_runtime::CatalogueRuntimeSnapshot>>);

impl FakeRuntime {
    fn over(entries: Vec<CatalogueEntry>, generation: u64) -> Arc<Self> {
        let catalogue = crate::domain::catalogue::resolve_catalogue(
            generation,
            vec![(SourceLayer::BuiltIn, entries)],
        )
        .snapshot;
        Arc::new(Self(Some(Arc::new(
            crate::application::provider_runtime::CatalogueRuntimeSnapshot {
                catalogue: Arc::new(catalogue),
                provider: Arc::new(SilentProvider),
            },
        ))))
    }
    fn none() -> Arc<Self> {
        Arc::new(Self(None))
    }
}

impl RuntimeSnapshotSource for FakeRuntime {
    fn current_runtime(
        &self,
    ) -> Option<Arc<crate::application::provider_runtime::CatalogueRuntimeSnapshot>> {
        self.0.clone()
    }
}

struct FromStore(CatalogueSnapshotStore);

impl EffortVocabularySource for FromStore {
    fn effort_vocabulary(&self, model: &str) -> Option<Vec<EffortLevel>> {
        let reference = ModelRef::parse_qualified(model).ok()?;
        let snapshot = self.0.current();
        let entry = snapshot.find(&reference)?;
        Some(
            entry
                .model
                .capabilities
                .effort_levels
                .iter()
                .filter_map(|level| EffortLevel::parse(level))
                .collect(),
        )
    }
}

#[derive(Default)]
struct FakeLoop {
    model: String,
    limits: ModelLimits,
    effort: Option<EffortLevel>,
}

impl EffortRuntime for FakeLoop {
    fn effort(&self) -> Option<EffortLevel> {
        self.effort
    }
    fn startup_effort(&self) -> Option<EffortLevel> {
        None
    }
    fn apply_effort(&mut self, level: Option<EffortLevel>) {
        self.effort = level;
    }
}

impl ModelRuntime for FakeLoop {
    fn model(&self) -> &str {
        &self.model
    }
    fn apply_model(&mut self, model: String, limits: ModelLimits) {
        self.model = model;
        self.limits = limits;
    }
}

struct Rig {
    source: Arc<FakeSource>,
    loader: Arc<Loader>,
    store: CatalogueSnapshotStore,
    use_case: ChangeActiveModel,
}

fn rig(entries: Vec<CatalogueEntry>, runtime: Arc<FakeRuntime>) -> Rig {
    rig_persisting(entries, runtime, Arc::new(RecordedDefaults::default()))
}

fn rig_persisting(
    entries: Vec<CatalogueEntry>,
    runtime: Arc<FakeRuntime>,
    persistence: Arc<RecordedDefaults>,
) -> Rig {
    let source = Arc::new(FakeSource(Mutex::new(entries)));
    let loader = Arc::new(Loader {
        source: source.clone(),
        loads: Mutex::new(0),
    });
    let store = CatalogueSnapshotStore::empty();
    let effort = Arc::new(ChangeReasoningEffort::new(
        Arc::new(FromStore(store.clone())),
        persistence.clone(),
    ));
    let use_case =
        ChangeActiveModel::new(loader.clone(), store.clone(), runtime, effort, persistence);
    Rig {
        source,
        loader,
        store,
        use_case,
    }
}

#[test]
fn plan_reads_explicit_limits_from_the_generation_it_just_published() {
    let entries = vec![entry("acme", "limited", Some((50, 1234)))];
    let rig = rig(entries.clone(), FakeRuntime::over(entries, 7));
    let plan = rig.use_case.plan("acme/limited");
    assert_eq!(
        plan.verdict,
        ModelSelectionVerdict::Runnable {
            provider: "acme".into(),
            generation: 7,
        }
    );
    assert_eq!(
        plan.limits,
        ModelLimits {
            max_output_tokens: Some(50),
            context_window: Some(1234),
        }
    );
    assert_eq!(rig.store.current().generation(), 1, "the plan published");
    assert_eq!(*rig.loader.loads.lock().unwrap(), 1);
}

#[test]
fn each_limit_clamps_only_when_declared_explicitly() {
    // maxTokens declared, contextWindow synthesized: only the cap clamps.
    let mut capped = entry("acme", "capped", Some((50, 128_000)));
    capped.model.capabilities.context_window_explicit = false;
    // contextWindow declared, maxTokens synthesized: only the window clamps.
    let mut windowed = entry("acme", "windowed", Some((4096, 1234)));
    windowed.model.capabilities.max_output_tokens_explicit = false;
    let rig = rig(vec![capped, windowed], FakeRuntime::none());
    assert_eq!(
        rig.use_case.plan("acme/capped").limits,
        ModelLimits {
            max_output_tokens: Some(50),
            context_window: None,
        }
    );
    assert_eq!(
        rig.use_case.plan("acme/windowed").limits,
        ModelLimits {
            max_output_tokens: None,
            context_window: Some(1234),
        }
    );
}

#[test]
fn synthesized_defaults_never_clamp() {
    let rig = rig(vec![entry("acme", "plain", None)], FakeRuntime::none());
    let plan = rig.use_case.plan("acme/plain");
    assert_eq!(plan.limits, ModelLimits::default());
    assert_eq!(plan.verdict, ModelSelectionVerdict::NoRuntime);
}

#[test]
fn a_model_added_since_the_last_read_is_switchable_without_a_refresh() {
    let rig = rig(vec![], FakeRuntime::none());
    assert_eq!(rig.use_case.plan("acme/new").limits, ModelLimits::default());
    rig.source
        .0
        .lock()
        .unwrap()
        .push(entry("acme", "new", Some((10, 20))));
    assert_eq!(
        rig.use_case.plan("acme/new").limits,
        ModelLimits {
            max_output_tokens: Some(10),
            context_window: Some(20),
        }
    );
    assert_eq!(*rig.loader.loads.lock().unwrap(), 2);
}

#[test]
fn an_unparsable_reference_is_unknown_with_no_limits_and_still_switches() {
    let rig = rig(vec![], FakeRuntime::none());
    let mut runtime = FakeLoop::default();
    let switched = rig.use_case.execute(&mut runtime, "not a reference");
    assert_eq!(
        switched.plan.verdict,
        ModelSelectionVerdict::Unknown {
            reference: "not a reference".into()
        }
    );
    assert_eq!(runtime.model, "not a reference");
    assert_eq!(runtime.limits, ModelLimits::default());
}

#[test]
fn execute_applies_model_and_limits_and_resets_effort_for_the_new_model() {
    let entries = vec![entry("acme", "limited", Some((50, 1234)))];
    let rig = rig(entries.clone(), FakeRuntime::over(entries, 1));
    let mut runtime = FakeLoop {
        model: "old/model".into(),
        effort: Some(EffortLevel::XHigh),
        ..Default::default()
    };
    let switched = rig.use_case.execute(&mut runtime, "acme/limited");
    assert_eq!(runtime.model, "acme/limited");
    assert_eq!(runtime.limits.max_output_tokens, Some(50));
    assert_eq!(
        runtime.effort,
        Some(EffortLevel::Low),
        "reset for the new model"
    );
    assert!(switched.effort_changed);
    assert!(matches!(
        switched.plan.verdict,
        ModelSelectionVerdict::Runnable { ref provider, generation: 1 } if provider == "acme"
    ));
    // The reset went through the effort use case: a later change on the
    // new model is validated against its vocabulary.
    let error = rig
        .use_case
        .effort
        .execute(
            &mut runtime,
            &EffortChangeRequest {
                model: "acme/limited".into(),
                level: "xhigh".into(),
                persist: None,
            },
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("valid levels: low, medium, high")
    );
}

#[test]
fn a_not_runnable_verdict_is_carried_and_the_switch_still_proceeds() {
    let reference = ModelRef::parse_qualified("acme/keyless").unwrap();
    let mut keyless = entry("acme", "keyless", None);
    keyless.model.availability = Availability::unavailable(
        crate::domain::catalogue::AvailabilityStatus::Available,
        vec![UnavailableReason::MissingCredential],
    )
    .unwrap();
    let rig = rig(vec![keyless.clone()], FakeRuntime::over(vec![keyless], 1));
    let mut runtime = FakeLoop::default();
    let switched = rig.use_case.execute(&mut runtime, "acme/keyless");
    assert_eq!(runtime.model, "acme/keyless");
    assert_eq!(
        switched.plan.verdict,
        ModelSelectionVerdict::NotRunnable {
            reference,
            reasons: vec![UnavailableReason::MissingCredential],
        }
    );
}

#[test]
fn debug_does_not_expose_the_ports() {
    let rig = rig(vec![], FakeRuntime::none());
    assert_eq!(format!("{:?}", rig.use_case), "ChangeActiveModel { .. }");
}

// ── Persisting a default (#2024 S2) ─────────────────────────────────────────

#[test]
fn execute_with_default_records_the_qualified_id_before_applying() {
    let entries = vec![entry("acme", "m", None)];
    let persistence = Arc::new(RecordedDefaults::default());
    let rig = rig_persisting(
        entries.clone(),
        FakeRuntime::over(entries, 1),
        persistence.clone(),
    );
    let mut lp = FakeLoop {
        model: "old/model".into(),
        ..Default::default()
    };
    let switched = rig
        .use_case
        .execute_with_default(&mut lp, "acme/m", Some(DefaultScope::Local))
        .unwrap();
    assert_eq!(
        persistence.records.lock().unwrap().as_slice(),
        &[(
            DefaultScope::Local,
            "agents.defaults.model".to_string(),
            "acme/m".to_string()
        )]
    );
    assert_eq!(switched.persisted.unwrap().scope, DefaultScope::Local);
    assert_eq!(lp.model, "acme/m");
}

#[test]
fn a_bare_id_no_provider_resolves_is_not_recorded_and_not_applied() {
    let entries = vec![entry("acme", "m", None)];
    let persistence = Arc::new(RecordedDefaults::default());
    let rig = rig_persisting(
        entries.clone(),
        FakeRuntime::over(entries, 1),
        persistence.clone(),
    );
    let mut lp = FakeLoop {
        model: "old/model".into(),
        ..Default::default()
    };
    let error = rig
        .use_case
        .execute_with_default(&mut lp, "nobody", Some(DefaultScope::Local))
        .unwrap_err();
    assert_eq!(
        error,
        ModelSwitchError::Unqualified {
            model: "nobody".into()
        }
    );
    assert!(error.to_string().contains("provider/model"));
    assert!(persistence.records.lock().unwrap().is_empty());
    assert_eq!(lp.model, "old/model", "nothing applied");
}

#[test]
fn a_refused_record_leaves_the_session_untouched() {
    let entries = vec![entry("acme", "m", None)];
    let persistence = Arc::new(RecordedDefaults::refusing("overlay is not trusted"));
    let rig = rig_persisting(entries.clone(), FakeRuntime::over(entries, 1), persistence);
    let mut lp = FakeLoop {
        model: "old/model".into(),
        effort: Some(EffortLevel::High),
        ..Default::default()
    };
    let error = rig
        .use_case
        .execute_with_default(&mut lp, "acme/m", Some(DefaultScope::Local))
        .unwrap_err();
    assert_eq!(
        error,
        ModelSwitchError::Persist {
            model: "acme/m".into(),
            scope: DefaultScope::Local,
            reason: "overlay is not trusted".into(),
        }
    );
    assert!(error.to_string().contains("local default"));
    assert_eq!(lp.model, "old/model");
    assert_eq!(lp.effort, Some(EffortLevel::High), "no effort reset either");
}

#[test]
fn without_a_scope_nothing_is_recorded_and_execute_is_unchanged() {
    let entries = vec![entry("acme", "m", None)];
    let persistence = Arc::new(RecordedDefaults::refusing("must not be called"));
    let rig = rig_persisting(entries.clone(), FakeRuntime::over(entries, 1), persistence);
    let mut lp = FakeLoop {
        model: "old/model".into(),
        ..Default::default()
    };
    let switched = rig
        .use_case
        .execute_with_default(&mut lp, "acme/m", None)
        .unwrap();
    assert_eq!(switched.persisted, None);
    assert_eq!(rig.use_case.execute(&mut lp, "acme/m").persisted, None);
}

#[test]
fn a_provider_the_published_catalogue_does_not_know_is_not_recorded_and_not_applied() {
    let entries = vec![entry("acme", "m", None)];
    let persistence = Arc::new(RecordedDefaults::default());
    let rig = rig_persisting(
        entries.clone(),
        FakeRuntime::over(entries, 1),
        persistence.clone(),
    );
    let mut lp = FakeLoop {
        model: "old/model".into(),
        ..Default::default()
    };
    let error = rig
        .use_case
        .execute_with_default(&mut lp, "nobody/x", Some(DefaultScope::Local))
        .unwrap_err();
    assert_eq!(
        error,
        ModelSwitchError::UnknownProvider {
            model: "nobody/x".into(),
            provider: "nobody".into(),
        }
    );
    assert!(error.to_string().contains("`nobody`"));
    assert!(persistence.records.lock().unwrap().is_empty());
    assert_eq!(lp.model, "old/model", "nothing applied");
    // A known provider with an id the catalogue does not enumerate is
    // recorded: open-router prefixes accept ids the catalogue cannot list.
    let switched = rig
        .use_case
        .execute_with_default(&mut lp, "acme/unlisted", Some(DefaultScope::Local))
        .unwrap();
    assert!(matches!(
        switched.plan.verdict,
        ModelSelectionVerdict::Unknown { .. }
    ));
    assert_eq!(persistence.records.lock().unwrap()[0].2, "acme/unlisted");
    assert_eq!(lp.model, "acme/unlisted");
}
