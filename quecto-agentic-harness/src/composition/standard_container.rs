//! The standard container's graph (#2024 S4e): `quecto container init`
//! and `quecto container status` over the embedded bundle, the checkout's
//! git origin, the launch policy's effective container-config roster and
//! lookup, the S4b script preflight — and the overlay write mapped onto
//! the configuration capability's patch use case, in the manner of
//! `catalogue_defaults.rs`: composition adapts the concrete
//! `PatchConfiguration` handle to the environments capability's
//! `ContainerConfigPersistence` port so the two capabilities never name
//! each other, and the entry init writes goes through the one write path
//! (exclusive hold, layer and merge validation, trust recorded for
//! exactly the bytes written, an untrusted overlay refused). The mapping
//! touches no file, lock or process itself, which is why it lives here
//! and not in infrastructure.

use std::path::Path;
use std::sync::Arc;

use crate::application::configuration::dto::{ConfigLayer, ConfigPatch, ConfigSelection};
use crate::application::configuration::use_cases::PatchConfiguration;
use crate::application::environments::dto::PersistedContainerConfig;
use crate::application::environments::ports::ContainerConfigPersistence;
use crate::application::environments::use_cases::{ContainerStatus, InitialiseStandardContainer};
use crate::infrastructure::processes::containers::standard::assets::EmbeddedStandardAssets;
use crate::infrastructure::processes::containers::standard::workspace_origin::GitWorkspaceOrigin;
use crate::interface::cli::configuration_handles::ConfigurationEnvironment;

/// The overlay's `container_configs.<name>` written through the patch
/// handle bound to one run's selection.
pub struct OverlayContainerConfigWriter {
    patch: Arc<PatchConfiguration>,
    selection: ConfigSelection,
}

impl OverlayContainerConfigWriter {
    pub fn new(patch: Arc<PatchConfiguration>, selection: ConfigSelection) -> Self {
        Self { patch, selection }
    }
}

impl ContainerConfigPersistence for OverlayContainerConfigWriter {
    fn persist(
        &self,
        name: &str,
        entry: serde_json::Value,
    ) -> Result<PersistedContainerConfig, String> {
        let receipt = self
            .patch
            .execute(ConfigPatch {
                selection: self.selection.clone(),
                layer: ConfigLayer::Overlay,
                key_path: format!("container_configs.{name}"),
                value: entry,
            })
            .map_err(|error| error.to_string())?;
        Ok(PersistedContainerConfig {
            path: receipt.path,
            created: receipt.created,
        })
    }

    fn location(&self) -> Option<std::path::PathBuf> {
        self.selection.overlay_path().map(Path::to_path_buf)
    }
}

impl std::fmt::Debug for OverlayContainerConfigWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OverlayContainerConfigWriter")
            .field("selection", &self.selection)
            .finish_non_exhaustive()
    }
}

/// The init `quecto container init` invokes, over the run's own
/// configuration selection with trust recorded under `base_dir` (where
/// the state dir is placed too). `main` hands this builder to the CLI.
pub fn build_standard_container_init(
    base_dir: &Path,
    selection: &ConfigSelection,
) -> Arc<InitialiseStandardContainer> {
    Arc::new(InitialiseStandardContainer::new(
        Arc::new(EmbeddedStandardAssets),
        Arc::new(GitWorkspaceOrigin),
        super::container_configs::build_container_config_roster(base_dir, Some(selection.clone())),
        build_container_config_persistence(base_dir, selection),
        base_dir.to_path_buf(),
    ))
}

/// The status `quecto container status` invokes, over the same layers.
pub fn build_container_status(
    base_dir: &Path,
    selection: &ConfigSelection,
) -> Arc<ContainerStatus> {
    Arc::new(ContainerStatus::new(
        Arc::new(EmbeddedStandardAssets),
        super::container_configs::build_container_config_roster(base_dir, Some(selection.clone())),
        super::environments::build_container_config_lookup(base_dir, Some(selection.clone())),
        super::environments::build_container_runtime_preflight(),
    ))
}

/// The persistence port adapter alone, for the contract suite.
pub fn build_container_config_persistence(
    base_dir: &Path,
    selection: &ConfigSelection,
) -> Arc<dyn ContainerConfigPersistence> {
    let handles = super::configuration::build_configuration_handles(&ConfigurationEnvironment {
        base_dir: base_dir.to_path_buf(),
        prompt_for_trust: false,
    });
    Arc::new(OverlayContainerConfigWriter::new(
        handles.patch,
        selection.clone(),
    ))
}

/// The asset store port adapter alone, for the contract suite.
pub fn build_container_asset_store()
-> Arc<dyn crate::application::environments::ports::ContainerAssetStore> {
    Arc::new(EmbeddedStandardAssets)
}

/// The origin port adapter alone, for the contract suite.
pub fn build_workspace_origin() -> Arc<dyn crate::application::environments::ports::WorkspaceOrigin>
{
    Arc::new(GitWorkspaceOrigin)
}
