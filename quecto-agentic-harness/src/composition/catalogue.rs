//! Catalogue composition (#1845, #1848, #1847, #1846, #1849): the catalogue
//! use cases over the file-backed inputs loaders and the process-wide
//! snapshot and runtime stores of one base directory, and the reload of a
//! run's configuration files over the ADR-0002 gate and the injected
//! provider-runtime builder startup composed through.
//! `main` hands [`build_catalogue_handles`] to the CLI entry point; the
//! dispatch loop holds the controller it receives.

use std::sync::Arc;

use crate::application::catalogue::use_cases::{
    ChangeActiveModel, ChangeReasoningEffort, ListModels, RefreshCatalogueSources,
    ReloadRuntimeConfiguration,
};
use crate::infrastructure::catalogue_inputs::FileCatalogueInputs;
use crate::infrastructure::catalogue_refresh_inputs::FileRefreshInputs;
use crate::infrastructure::catalogue_registry::{
    PublishedEffortVocabulary, runtime_store_for, snapshot_store_for,
};
use crate::infrastructure::runtime_configuration::FileRuntimeConfiguration;
use crate::interface::uds::catalogue::list_models_controller::ListModelsController;

use crate::interface::cli::catalogue_handles::{CatalogueHandles, RuntimeConfigurationInputs};

/// The catalogue handles one loop holds, over the shared snapshot store of
/// `base_dir`. With `runtime`, the reload use case watches the run's config
/// file and `models.json` and rebuilds through the provider-runtime builder
/// startup was injected with; without it (rigs, the `models` CLI) reload is
/// unconfigured.
pub fn build_catalogue_handles(
    base_dir: &std::path::Path,
    runtime: Option<&RuntimeConfigurationInputs>,
) -> CatalogueHandles {
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
    let refresh = Arc::new(RefreshCatalogueSources::new(
        Arc::new(FileRefreshInputs::new(base_dir)),
        snapshot_store_for(base_dir),
    ));
    let reload = Arc::new(match runtime {
        Some(runtime) => {
            ReloadRuntimeConfiguration::new(Box::new(FileRuntimeConfiguration::seeded(
                super::configuration::watched_config_sources(base_dir, &runtime.selection),
                base_dir.to_path_buf(),
                super::configuration::build_config_loader(
                    base_dir,
                    runtime.selection.clone(),
                    runtime.env_overrides.clone(),
                ),
                runtime.http_client.clone(),
                runtime.provider_runtime,
            )))
        }
        None => ReloadRuntimeConfiguration::unconfigured(),
    });
    CatalogueHandles {
        list_models: Arc::new(ListModelsController::new(list_models)),
        effort,
        model,
        refresh,
        reload,
    }
}

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "catalogue_refresh_tests.rs"]
mod refresh_tests;

#[cfg(test)]
#[path = "catalogue_resolve_tests.rs"]
mod resolve_tests;

/// The `list_models` wire response for `base_dir` as the composed loop
/// would serve it: rigs and BDD steps that used to call the interface's
/// `list_models_data` (retired, #1845) read through the same builder and presenter.
#[cfg(any(test, feature = "test-support"))]
pub fn list_models_wire_for(base_dir: &std::path::Path) -> serde_json::Value {
    crate::interface::uds::catalogue::list_models_presenter::render(
        &build_catalogue_handles(base_dir, None).list_models.list(),
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
    build_catalogue_handles(base_dir, None)
        .model
        .startup_limits(model)
}
