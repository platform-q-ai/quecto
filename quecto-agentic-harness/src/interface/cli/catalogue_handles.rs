//! The catalogue handles a dispatch loop holds (#1845). Declared here as a
//! plain struct of controller handles; composition (`composition::catalogue`)
//! fills it through the builder `main` hands to the CLI. The interface never
//! constructs the use case behind it.

use std::sync::Arc;

use crate::application::catalogue::use_cases::{ChangeActiveModel, ChangeReasoningEffort};
use crate::interface::uds::catalogue::list_models_controller::ListModelsController;

/// The composed `list_models` controller a dispatch loop holds.
pub type ListModelsHandle = Arc<ListModelsController>;

#[derive(Clone, Debug)]
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
