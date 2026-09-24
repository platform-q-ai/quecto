//! Steps for `active_model.feature` (#1847): the change-active-model use
//! case over fake ports, plus the UDS `set_model` reply's selection verdict.

use std::sync::{Arc, Mutex};

use super::uds_steps::find_agent_response;
use super::*;
use quecto::application::catalogue::dto::{ModelLimits, ModelSelectionVerdict, ModelSwitched};
use quecto::application::catalogue::ports::{
    CatalogueInputsLoader, EffortRuntime, EffortVocabularySource, LoadedCatalogueInputs,
    ModelRuntime, RuntimeSnapshotSource,
};
use quecto::application::catalogue::use_cases::{ChangeActiveModel, ChangeReasoningEffort};
use quecto::application::catalogue::{
    CatalogueSnapshotStore, CatalogueSource, CredentialStatusPort, SourceEntries,
};
use quecto::application::provider_runtime::CatalogueRuntimeSnapshot;
use quecto::domain::catalogue::{
    AuthIdentity, Availability, CatalogueEntry, ModelCapabilities, ModelCost, ModelDescriptor,
    ModelRef, ProviderDescriptor, SourceLayer, TransportKind, UnavailableReason,
};
use quecto::domain::provider::EffortLevel;

#[derive(Debug, Default)]
pub struct ActiveModelState {
    entries: Vec<CatalogueEntry>,
    denied: Vec<String>,
    store: Option<CatalogueSnapshotStore>,
    runtime: Option<Arc<CatalogueRuntimeSnapshot>>,
    loop_model: String,
    loop_limits: ModelLimits,
    loop_effort: Option<EffortLevel>,
    switched: Option<ModelSwitched>,
}

fn am_entry(qualified: &str, limits: Option<(u32, u32)>) -> CatalogueEntry {
    let reference = ModelRef::parse_qualified(qualified).unwrap();
    let (max_output_tokens, context_window) = limits.unwrap_or((4096, 128_000));
    CatalogueEntry {
        provider: ProviderDescriptor {
            id: reference.provider().clone(),
            display_name: None,
            transport: TransportKind::OpenAiCompletions,
            auth: AuthIdentity::ApiKey,
        },
        model: ModelDescriptor {
            reference,
            display_name: None,
            capabilities: ModelCapabilities {
                effort_levels: vec!["low".into(), "medium".into(), "high".into()],
                input_modalities: vec!["text".into()],
                context_window,
                max_output_tokens,
                context_window_explicit: limits.is_some(),
                max_output_tokens_explicit: limits.is_some(),
                reasoning: true,
                cost: ModelCost::default(),
            },
            availability: Availability::runnable(),
        },
    }
}

struct AmSource(Vec<CatalogueEntry>);

impl CatalogueSource for AmSource {
    fn id(&self) -> &str {
        "inputs"
    }
    fn layer(&self) -> SourceLayer {
        SourceLayer::UserDefined
    }
    fn load(&self) -> Result<SourceEntries, String> {
        Ok(self.0.clone().into())
    }
}

struct AmCredentials(Vec<String>);

impl CredentialStatusPort for AmCredentials {
    fn credential_available(&self, entry: &CatalogueEntry) -> bool {
        !self.0.contains(&entry.reference().qualified_id())
    }
}

struct AmLoaded {
    source: AmSource,
    credentials: AmCredentials,
}

impl LoadedCatalogueInputs for AmLoaded {
    fn sources(&self) -> Vec<&dyn CatalogueSource> {
        vec![&self.source]
    }
    fn credentials(&self) -> &dyn CredentialStatusPort {
        &self.credentials
    }
}

/// Loads whatever the world's inputs hold at the time of the load.
struct AmLoader(Arc<Mutex<(Vec<CatalogueEntry>, Vec<String>)>>);

impl CatalogueInputsLoader for AmLoader {
    fn load(&self) -> Box<dyn LoadedCatalogueInputs> {
        let (entries, denied) = self.0.lock().unwrap().clone();
        Box::new(AmLoaded {
            source: AmSource(entries),
            credentials: AmCredentials(denied),
        })
    }
}

struct AmRuntime(Option<Arc<CatalogueRuntimeSnapshot>>);

impl RuntimeSnapshotSource for AmRuntime {
    fn current_runtime(&self) -> Option<Arc<CatalogueRuntimeSnapshot>> {
        self.0.clone()
    }
}

#[derive(Debug)]
struct AmProvider;

impl quecto::application::providers::ports::LlmProvider for AmProvider {
    fn name(&self) -> &str {
        "router"
    }
    fn chat<'a>(
        &'a self,
        _request: quecto::application::providers::ports::ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        quecto::domain::message::LlmResponse,
                        quecto::domain::error::DomainError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async {
            Err(quecto::domain::error::DomainError::Provider(
                "silent".into(),
            ))
        })
    }
}

struct AmStoreVocabulary(CatalogueSnapshotStore);

impl EffortVocabularySource for AmStoreVocabulary {
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
                .filter_map(|l| EffortLevel::parse(l))
                .collect(),
        )
    }
}

/// The loop as the world sees it.
struct AmLoop<'a>(&'a mut ActiveModelState);

impl EffortRuntime for AmLoop<'_> {
    fn effort(&self) -> Option<EffortLevel> {
        self.0.loop_effort
    }
    fn startup_effort(&self) -> Option<EffortLevel> {
        None
    }
    fn apply_effort(&mut self, level: Option<EffortLevel>) {
        self.0.loop_effort = level;
    }
}

impl ModelRuntime for AmLoop<'_> {
    fn model(&self) -> &str {
        &self.0.loop_model
    }
    fn apply_model(&mut self, model: String, limits: ModelLimits) {
        self.0.loop_model = model;
        self.0.loop_limits = limits;
    }
    fn route_check(&self, _model: &str) -> quecto::application::providers::ports::RouteCheck {
        // These scenarios switch among catalogue models on configured providers.
        quecto::application::providers::ports::RouteCheck::Routable
    }
}

fn am_store(state: &mut ActiveModelState) -> CatalogueSnapshotStore {
    state
        .store
        .get_or_insert_with(CatalogueSnapshotStore::empty)
        .clone()
}

#[given(
    expr = "a catalogue input defining model {string} with max tokens {int} and context window {int}"
)]
fn given_input_with_limits(world: &mut QuectoWorld, qualified: String, max: u32, window: u32) {
    world
        .active_model
        .entries
        .push(am_entry(&qualified, Some((max, window))));
}

#[given(expr = "a catalogue input defining model {string} with no declared limits")]
fn given_input_without_limits(world: &mut QuectoWorld, qualified: String) {
    world.active_model.entries.push(am_entry(&qualified, None));
}

#[when(
    expr = "the catalogue input additionally defines model {string} with max tokens {int} and context window {int}"
)]
fn when_input_grows(world: &mut QuectoWorld, qualified: String, max: u32, window: u32) {
    world
        .active_model
        .entries
        .push(am_entry(&qualified, Some((max, window))));
}

#[given(expr = "no credential is available for {string}")]
fn given_no_credential_for(world: &mut QuectoWorld, qualified: String) {
    world.active_model.denied.push(qualified);
}

/// A runtime generation composed over the inputs as they stand now.
#[given("a runtime generation composed over those inputs")]
fn given_runtime_over_inputs(world: &mut QuectoWorld) {
    let entries = world.active_model.entries.clone();
    let denied = world.active_model.denied.clone();
    let resolved = quecto::application::catalogue::ResolveCatalogueUseCase.resolve_and_publish(
        &[&AmSource(entries)],
        &AmCredentials(denied),
        &am_store(&mut world.active_model),
    );
    world.active_model.runtime = Some(Arc::new(CatalogueRuntimeSnapshot {
        catalogue: resolved.snapshot,
        provider: Arc::new(AmProvider),
        admission_binding_diagnostic: Default::default(),
    }));
}

#[given(expr = "the loop's effort is {string}")]
fn given_loop_effort(world: &mut QuectoWorld, level: String) {
    world.active_model.loop_effort = EffortLevel::parse(&level);
}

#[when(expr = "the active model is changed to {string}")]
fn when_active_model_changed(world: &mut QuectoWorld, model: String) {
    let store = am_store(&mut world.active_model);
    let inputs = Arc::new(AmLoader(Arc::new(Mutex::new((
        world.active_model.entries.clone(),
        world.active_model.denied.clone(),
    )))));
    let defaults = Arc::new(quecto::application::catalogue::ports::RecordedDefaults::default());
    let effort = Arc::new(ChangeReasoningEffort::new(
        Arc::new(AmStoreVocabulary(store.clone())),
        defaults.clone(),
    ));
    let use_case = ChangeActiveModel::new(
        inputs,
        store,
        Arc::new(AmRuntime(world.active_model.runtime.clone())),
        effort,
        defaults,
    );
    let switched = use_case.execute(&mut AmLoop(&mut world.active_model), &model);
    world.active_model.switched = Some(switched);
}

#[then(expr = "the loop runs on {string} with max tokens {int} and context window {int}")]
fn then_loop_runs_with_limits(world: &mut QuectoWorld, model: String, max: u32, window: usize) {
    assert_eq!(world.active_model.loop_model, model);
    assert_eq!(
        world.active_model.loop_limits,
        ModelLimits {
            max_output_tokens: Some(max),
            context_window: Some(window),
        }
    );
}

#[then(expr = "the loop runs on {string} with no declared limits")]
fn then_loop_runs_unclamped(world: &mut QuectoWorld, model: String) {
    assert_eq!(world.active_model.loop_model, model);
    assert_eq!(world.active_model.loop_limits, ModelLimits::default());
}

#[then(expr = "the published catalogue generation is {int}")]
fn then_generation_is(world: &mut QuectoWorld, generation: u64) {
    assert_eq!(
        am_store(&mut world.active_model).current().generation(),
        generation
    );
}

#[then("the switch verdict is not runnable because a credential is missing")]
fn then_verdict_not_runnable(world: &mut QuectoWorld) {
    let switched = world.active_model.switched.as_ref().expect("no switch");
    match &switched.plan.verdict {
        ModelSelectionVerdict::NotRunnable { reasons, .. } => {
            assert!(
                reasons.contains(&UnavailableReason::MissingCredential),
                "{reasons:?}"
            )
        }
        other => panic!("expected not runnable, got {other:?}"),
    }
}

#[then(expr = "the switch verdict is unknown model {string}")]
fn then_verdict_unknown(world: &mut QuectoWorld, reference: String) {
    let switched = world.active_model.switched.as_ref().expect("no switch");
    assert_eq!(
        switched.plan.verdict,
        ModelSelectionVerdict::Unknown { reference }
    );
}

#[then(expr = "the loop's effort is {string}")]
fn then_loop_effort(world: &mut QuectoWorld, level: String) {
    assert_eq!(world.active_model.loop_effort, EffortLevel::parse(&level));
}

#[then(expr = "the set_model response selection status should be {string} on provider {string}")]
fn then_set_model_selection(world: &mut QuectoWorld, status: String, provider: String) {
    let resp = find_agent_response(world, "set_model").expect("no set_model response");
    assert_eq!(resp["data"]["selection"]["status"], status, "{resp}");
    assert_eq!(resp["data"]["selection"]["provider"], provider, "{resp}");
}
