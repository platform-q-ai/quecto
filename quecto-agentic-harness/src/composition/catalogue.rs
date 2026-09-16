//! Catalogue composition (#1845): the list use case over the file-backed
//! inputs loader and the process-wide snapshot store of one base directory.
//! `main` hands [`build_catalogue_handles`] to the CLI entry point; the
//! dispatch loop holds the controller it receives.

use std::sync::Arc;

use crate::application::catalogue::use_cases::ListModels;
use crate::infrastructure::catalogue_inputs::FileCatalogueInputs;
use crate::infrastructure::catalogue_registry::snapshot_store_for;
use crate::interface::uds::catalogue::list_models_controller::ListModelsController;

pub use crate::interface::cli::catalogue_handles::CatalogueHandles;

/// The catalogue handles one loop holds, over the shared snapshot store of
/// `base_dir`.
pub fn build_catalogue_handles(base_dir: &std::path::Path) -> CatalogueHandles {
    let list_models = Arc::new(ListModels::new(
        Arc::new(FileCatalogueInputs::new(base_dir)),
        snapshot_store_for(base_dir),
    ));
    CatalogueHandles {
        list_models: Arc::new(ListModelsController::new(list_models)),
    }
}

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod tests;

/// The `list_models` wire response for `base_dir` as the composed loop
/// would serve it: rigs and BDD steps that used to call the interface's
/// `list_models_data` (retired, #1845) read through the same builder and presenter.
#[cfg(any(test, feature = "test-support"))]
pub fn list_models_wire_for(base_dir: &std::path::Path) -> serde_json::Value {
    crate::interface::uds::catalogue::list_models_presenter::render(
        &build_catalogue_handles(base_dir).list_models.list(),
    )
}
