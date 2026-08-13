use crate::components::effort_selector::EffortSelector;
use crate::components::model_selector::{ModelEntry, ModelSelector};

/// Global selector-overlay half of the inference flow (#1463): the model /
/// effort selector overlays and registry are app-chrome shared across tabs;
/// the per-connection model/effort STATE lives in [`ConnInference`] on
/// `ConnectionState`.
#[derive(Default)]
pub(super) struct InferenceFlow {
    /// The model selector component (created on demand, pushed onto overlay stack).
    pub(super) model_selector: Option<ModelSelector>,
    pub(super) model_registry: ModelRegistry,
    /// The effort selector overlay (#1067), opened by bare `/effort`.
    pub(super) effort_selector: Option<EffortSelector>,
}

/// Per-connection inference state (#1463): the model/effort the TAB's agent
/// currently runs with, scoped to its connection.
#[derive(Default)]
pub(crate) struct ConnInference {
    pub(crate) current_model: Option<String>,
    /// Active effort level (`None` = default), for selector marker + footer (#1067).
    pub(crate) current_effort: Option<String>,
    /// Effort vocabulary for the active provider, reported by the agent in
    /// `get_state` (`effortLevels`) — never re-derived locally (#1067).
    pub(crate) effort_levels: Vec<String>,
}

/// Model registry owned by the selector flow (#997).
#[derive(Default)]
pub(crate) struct ModelRegistry {
    /// Models parsed from the last `list_models` response (may be empty).
    pub(super) entries: Vec<ModelEntry>,
    /// A selector open is deferred until the fresh list arrives (ADR-0002).
    pub(super) open_pending: bool,
}
