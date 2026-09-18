//! The container-config inventory read the way a launch reads it (#2024
//! S4c): the environments capability's [`ContainerConfigRoster`] port over
//! the launch policy's [`EffectiveContainerConfigs`] port, so `agent_cmd
//! get_container_configs`, the spawn description's roster line and
//! `spawn container: true` agree on which entries exist for the launching
//! agent's checkout, which the overlay declared, and whether the overlay
//! was withheld.
use std::path::PathBuf;
use std::sync::Arc;

use crate::application::environments::dto::{ContainerConfigEntry, ContainerConfigLayer};
use crate::application::environments::ports::{ContainerConfigRoster, ContainerConfigRosterReport};
use crate::application::subagents::dto::ContainerConfigSource;
use crate::application::subagents::ports::EffectiveContainerConfigs;

/// Composition's cheap probe of the files the roster is read from: the
/// configuration layers and the trust record. Its value changes when any
/// of them is written, created or removed.
pub type RosterRevisionProbe = Arc<dyn Fn() -> String + Send + Sync>;

pub struct EffectiveConfigRoster {
    configs: Arc<dyn EffectiveContainerConfigs>,
    revision: RosterRevisionProbe,
}

impl EffectiveConfigRoster {
    pub fn new(configs: Arc<dyn EffectiveContainerConfigs>, revision: RosterRevisionProbe) -> Self {
        Self { configs, revision }
    }
}

/// A revision token over `paths`: each file's modification time and
/// length (or its absence), in order. Metadata only — no file is read.
pub fn file_revision(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| match std::fs::metadata(path) {
            Ok(meta) => {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|since| since.as_nanos())
                    .unwrap_or_default();
                format!("{}@{modified}:{}", path.display(), meta.len())
            }
            Err(_) => format!("{}@absent", path.display()),
        })
        .collect::<Vec<_>>()
        .join(";")
}

impl ContainerConfigRoster for EffectiveConfigRoster {
    fn roster(&self) -> Result<ContainerConfigRosterReport, String> {
        let set = self
            .configs
            .effective_container_configs(&ContainerConfigSource::LaunchingAgent)
            .map_err(|error| error.to_string())?;
        Ok(ContainerConfigRosterReport {
            configs: set
                .configs
                .into_iter()
                .map(|config| ContainerConfigEntry {
                    problem: config.argv_problem().map(str::to_string),
                    name: config.name,
                    default: config.default,
                    layer: if config.repo_bound {
                        ContainerConfigLayer::Overlay
                    } else {
                        ContainerConfigLayer::Global
                    },
                    repository: config.repository,
                    joinable: !config.exec.is_empty(),
                })
                .collect(),
            overlay_withheld: set.overlay_withheld,
            diagnostics: set.diagnostics,
        })
    }

    fn revision(&self) -> String {
        (self.revision)()
    }
}

impl std::fmt::Debug for EffectiveConfigRoster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectiveConfigRoster")
            .finish_non_exhaustive()
    }
}
