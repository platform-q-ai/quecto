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

use serde_json::Value;

use crate::application::configuration::dto::{
    ConfigLayer, ConfigPatch, ConfigPatchError, ConfigSelection, OverlayState,
};
use crate::application::configuration::use_cases::{PatchConfiguration, ResolveEffectiveConfig};
use crate::application::environments::dto::{ContainerConfigDocument, PersistedContainerConfig};
use crate::application::environments::ports::ContainerConfigPersistence;
use crate::application::environments::use_cases::{ContainerStatus, InitialiseStandardContainer};
use crate::infrastructure::processes::containers::standard::assets::EmbeddedStandardAssets;
use crate::infrastructure::processes::containers::standard::workspace_origin::GitWorkspaceOrigin;
use crate::interface::cli::configuration_handles::ConfigurationEnvironment;

/// The overlay's `container_configs.<name>` written through the patch
/// handle bound to one run's selection.
pub struct OverlayContainerConfigWriter {
    patch: Arc<PatchConfiguration>,
    resolve: Arc<ResolveEffectiveConfig>,
    selection: ConfigSelection,
}

impl OverlayContainerConfigWriter {
    pub fn new(
        patch: Arc<PatchConfiguration>,
        resolve: Arc<ResolveEffectiveConfig>,
        selection: ConfigSelection,
    ) -> Self {
        Self {
            patch,
            resolve,
            selection,
        }
    }
}

impl ContainerConfigPersistence for OverlayContainerConfigWriter {
    /// The patch's own refusals, asked of the same resolution the patch
    /// validates against, in the patch's own words.
    fn check(&self) -> Result<(), String> {
        let Some(path) = self.selection.overlay_path().map(Path::to_path_buf) else {
            return Err(ConfigPatchError::NoOverlayLocation.to_string());
        };
        let effective = self
            .resolve
            .execute(&self.selection)
            .map_err(|error| error.to_string())?;
        match effective.sources.overlay.map(|report| report.state) {
            Some(OverlayState::Untrusted { fingerprint, .. }) => {
                Err(ConfigPatchError::UntrustedOverlay { path, fingerprint }.to_string())
            }
            Some(OverlayState::Refused { reason }) => {
                Err(ConfigPatchError::Refused { path, reason }.to_string())
            }
            Some(OverlayState::Applied | OverlayState::Absent) | None => Ok(()),
        }
    }

    /// One patch: `container_configs.<name>` alone, or — when another
    /// overlay entry's default label must go with it (#2035) — the whole
    /// `container_configs` section as the overlay declares it, that
    /// entry's label removed and ours set, so the merge validated is the
    /// one that lands (two patches would leave an invalid merge between
    /// them wherever the displaced entry was the only default).
    fn persist(
        &self,
        name: &str,
        entry: &ContainerConfigDocument,
        displace_default: Option<&str>,
    ) -> Result<PersistedContainerConfig, String> {
        let (key_path, value) = match displace_default {
            None => (format!("container_configs.{name}"), entry_document(entry)),
            Some(displaced) => {
                let mut section = self.overlay_section()?;
                let Some(other) = section.get_mut(displaced).and_then(Value::as_object_mut) else {
                    return Err(format!(
                        "cannot move the default label from container_configs.{displaced}: the repo-local overlay declares no such entry"
                    ));
                };
                other.remove("default");
                section.insert(name.to_string(), entry_document(entry));
                ("container_configs".to_string(), Value::Object(section))
            }
        };
        let receipt = self
            .patch
            .execute(ConfigPatch {
                selection: self.selection.clone(),
                layer: ConfigLayer::Overlay,
                key_path,
                value,
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

    /// The overlay's own `container_configs.<name>`, read from the applied
    /// overlay document the resolution reports (a withheld overlay is
    /// refused by `check` before this is consulted).
    fn existing(&self, name: &str) -> Result<Option<ContainerConfigDocument>, String> {
        Ok(self.overlay_section()?.get(name).map(document_entry))
    }
}

impl OverlayContainerConfigWriter {
    /// The overlay's `container_configs` section as the applied overlay
    /// document spells it (empty when absent), every entry verbatim.
    fn overlay_section(&self) -> Result<serde_json::Map<String, Value>, String> {
        let effective = self
            .resolve
            .execute(&self.selection)
            .map_err(|error| error.to_string())?;
        Ok(effective
            .overlay_document
            .as_ref()
            .and_then(|overlay| overlay.get("container_configs"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default())
    }
}

/// The inverse of [`entry_document`]: the entry as the overlay spells it,
/// read leniently (a missing or non-string argv element is dropped) —
/// the schema's own validation ran when the overlay was applied.
fn document_entry(document: &serde_json::Value) -> ContainerConfigDocument {
    let argv = |key: &str| -> Vec<String> {
        document
            .get(key)
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    ContainerConfigDocument {
        default: document
            .get("default")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        create: argv("create"),
        exec: argv("exec"),
        inspect: argv("inspect"),
        kill: argv("kill"),
        cleanup: argv("cleanup"),
    }
}

/// The entry as the configuration schema spells it: the `default` label
/// only when set (an absent label is how a non-default entry is written),
/// then each argv under its key.
pub fn entry_document(entry: &ContainerConfigDocument) -> serde_json::Value {
    let mut document = serde_json::Map::new();
    if entry.default {
        document.insert("default".into(), serde_json::Value::Bool(true));
    }
    for (key, argv) in entry.argvs() {
        document.insert(
            key.into(),
            serde_json::Value::Array(
                argv.iter()
                    .map(|arg| serde_json::Value::String(arg.clone()))
                    .collect(),
            ),
        );
    }
    serde_json::Value::Object(document)
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
        handles.resolve,
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

#[cfg(test)]
#[path = "standard_container_tests.rs"]
mod tests;
