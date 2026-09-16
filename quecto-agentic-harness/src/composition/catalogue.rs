//! Catalogue composition (#1845): the list use case over the file-backed
//! inputs loader and the process-wide snapshot store of one base directory.
//! `main` hands [`build_catalogue_handles`] to the CLI entry point; the
//! dispatch loop holds the controller it receives.

use std::sync::Arc;

use crate::application::catalogue::use_cases::{
    ChangeActiveModel, ChangeReasoningEffort, ListModels,
};
use crate::infrastructure::catalogue_inputs::FileCatalogueInputs;
use crate::infrastructure::catalogue_registry::{
    PublishedEffortVocabulary, runtime_store_for, snapshot_store_for,
};
use crate::interface::uds::catalogue::list_models_controller::ListModelsController;

use crate::interface::cli::catalogue_handles::CatalogueHandles;

/// The catalogue handles one loop holds, over the shared snapshot store of
/// `base_dir`.
pub fn build_catalogue_handles(base_dir: &std::path::Path) -> CatalogueHandles {
    let inputs: Arc<FileCatalogueInputs> = Arc::new(FileCatalogueInputs::new(base_dir));
    let list_models = Arc::new(ListModels::new(
        inputs.clone(),
        snapshot_store_for(base_dir),
    ));
    let effort = Arc::new(ChangeReasoningEffort::new(Arc::new(
        PublishedEffortVocabulary::for_base_dir(base_dir),
    )));
    let model = Arc::new(ChangeActiveModel::new(
        inputs,
        snapshot_store_for(base_dir),
        Arc::new(runtime_store_for(base_dir)),
        effort.clone(),
    ));
    CatalogueHandles {
        list_models: Arc::new(ListModelsController::new(list_models)),
        effort,
        model,
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

/// Resolve and publish the catalogue of `base_dir` from its on-disk inputs
/// and return the outcome, for rigs and BDD steps that inspect a resolve
/// (rejections, skipped records, source errors). The same loader and store
/// the composed use cases read through.
#[cfg(any(test, feature = "test-support"))]
pub fn resolve_catalogue_for(
    base_dir: &std::path::Path,
) -> crate::application::catalogue::ResolvedCatalogue {
    use crate::application::catalogue::ports::CatalogueInputsLoader;
    let inputs = FileCatalogueInputs::new(base_dir).load();
    crate::application::catalogue::ResolveCatalogueUseCase.resolve_and_publish(
        &inputs.sources(),
        inputs.credentials(),
        &snapshot_store_for(base_dir),
    )
}

/// The per-model limits the composed change-active-model use case reads for
/// `model` in `base_dir`, for rigs.
#[cfg(any(test, feature = "test-support"))]
pub fn published_model_limits_for(
    base_dir: &std::path::Path,
    model: &str,
) -> crate::application::catalogue::dto::ModelLimits {
    build_catalogue_handles(base_dir)
        .model
        .startup_limits(model)
}
