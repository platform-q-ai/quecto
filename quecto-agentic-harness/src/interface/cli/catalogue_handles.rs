//! The catalogue handles a dispatch loop holds (#1845). Declared here as a
//! plain struct of controller handles; composition (`composition::catalogue`)
//! fills it through the builder `main` hands to the CLI. The interface never
//! constructs the use case behind it.

use std::sync::Arc;

use crate::application::catalogue::use_cases::{
    ChangeActiveModel, ChangeReasoningEffort, RefreshCatalogueSources, ReloadRuntimeConfiguration,
};
use crate::interface::uds::catalogue::list_models_controller::ListModelsController;

/// The composed `list_models` controller a dispatch loop holds.
pub type ListModelsHandle = Arc<ListModelsController>;

#[derive(Clone)]
pub struct CatalogueHandles {
    /// List available models (#1845): answers the UDS `list_models` command.
    pub list_models: ListModelsHandle,
    /// Change reasoning effort (#1848): validates and applies `set_effort`,
    /// answers the `get_state` vocabulary, admits the startup default and
    /// the level carried across a model switch.
    pub effort: Arc<ChangeReasoningEffort>,
    /// Change the active model (#1847): plans and applies `set_model`, and
    /// supplies the startup model's limits.
    pub model: Arc<ChangeActiveModel>,
    /// Refresh model catalogue sources (#1846) and discover a provider's
    /// models (#1844): UDS `refresh_models` and CLI `models discover`.
    pub refresh: Arc<RefreshCatalogueSources>,
    /// Reload runtime configuration (#1849): the UDS `reload` command and
    /// the poll before every prompt and `set_model`. Unconfigured (every
    /// reload reports so) for a loop built without
    /// [`RuntimeConfigurationInputs`].
    pub reload: Arc<ReloadRuntimeConfiguration>,
}

impl std::fmt::Debug for CatalogueHandles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogueHandles")
            .field("list_models", &self.list_models)
            .field("effort", &self.effort)
            .field("model", &self.model)
            .field("refresh", &self.refresh)
            .field("reload", &self.reload)
            .finish()
    }
}

/// The run's reloadable configuration (#1849, #2024), as the interface
/// knows it at startup: the config layers the run selected, the
/// environment overrides it was loaded with, the HTTP client its providers
/// share and the injected provider-runtime builder startup composed
/// through. Composition builds the reload use case over them.
#[derive(Clone)]
pub struct RuntimeConfigurationInputs {
    pub selection: crate::application::configuration::dto::ConfigSelection,
    pub env_overrides: std::collections::HashMap<String, String>,
    pub http_client: reqwest::Client,
    pub provider_runtime: crate::infrastructure::runtime_configuration::ProviderRuntimeBuilder,
}

impl std::fmt::Debug for RuntimeConfigurationInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeConfigurationInputs")
            .field("selection", &self.selection)
            .field("env_overrides", &self.env_overrides)
            .finish_non_exhaustive()
    }
}

/// The effort fields of a dispatch loop's state snapshot (#1848).
pub(crate) fn effort_view(
    ctx: &super::uds::DispatchCtx<'_>,
) -> crate::interface::uds::catalogue::effort_presenter::EffortStateView {
    ctx.catalogue
        .effort_view(ctx.agent.effort(), ctx.session.model())
}

impl CatalogueHandles {
    /// The effort fields of a state snapshot: `current` plus `model`'s
    /// vocabulary from the change-reasoning-effort use case.
    pub fn effort_view(
        &self,
        current: Option<crate::domain::provider::EffortLevel>,
        model: &str,
    ) -> crate::interface::uds::catalogue::effort_presenter::EffortStateView {
        crate::interface::uds::catalogue::effort_presenter::EffortStateView::new(
            current,
            &self.effort.choices(model),
        )
    }
}
