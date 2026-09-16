//! The catalogue handles a dispatch loop holds (#1845). Declared here as a
//! plain struct of controller handles; composition (`composition::catalogue`)
//! fills it through the builder `main` hands to the CLI. The interface never
//! constructs the use case behind it.

use std::sync::Arc;

use crate::interface::uds::catalogue::list_models_controller::ListModelsController;

/// The composed `list_models` controller a dispatch loop holds.
pub type ListModelsHandle = Arc<ListModelsController>;

#[derive(Clone, Debug)]
pub struct CatalogueHandles {
    /// List available models (#1845): answers the UDS `list_models` command.
    pub list_models: ListModelsHandle,
}
