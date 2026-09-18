//! The container-config inventory read the way a launch reads it (#2024
//! S4c): the environments capability's [`ContainerConfigRoster`] port over
//! the launch policy's [`EffectiveContainerConfigs`] port, so `agent_cmd
//! get_container_configs`, the spawn description's roster line and
//! `spawn container: true` agree on which entries exist for the launching
//! agent's checkout, which the overlay declared, and whether the overlay
//! was withheld.
use std::sync::Arc;

use crate::application::environments::dto::{ContainerConfigEntry, ContainerConfigLayer};
use crate::application::environments::ports::{ContainerConfigRoster, ContainerConfigRosterReport};
use crate::application::subagents::dto::ContainerConfigSource;
use crate::application::subagents::ports::EffectiveContainerConfigs;

pub struct EffectiveConfigRoster {
    configs: Arc<dyn EffectiveContainerConfigs>,
}

impl EffectiveConfigRoster {
    pub fn new(configs: Arc<dyn EffectiveContainerConfigs>) -> Self {
        Self { configs }
    }
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
                })
                .collect(),
            overlay_withheld: set.overlay_withheld,
            diagnostics: set.diagnostics,
        })
    }
}

impl std::fmt::Debug for EffectiveConfigRoster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectiveConfigRoster")
            .finish_non_exhaustive()
    }
}
