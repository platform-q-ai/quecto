//! The doctor's target resolved the way a launch resolves it (#2024 S4b):
//! the environments capability's [`ContainerConfigLookup`] port over the
//! composed launch-policy selection, so `quecto container doctor` and
//! `spawn container: true` agree on which entry the working directory's
//! effective configuration labels as default, which names exist, and
//! which overlay diagnostics apply.
use std::sync::Arc;

use crate::application::environments::dto::{ContainerRuntimeTarget, DiagnosableContainerConfig};
use crate::application::environments::ports::ContainerConfigLookup;
use crate::application::subagents::dto::{
    ContainerConfigSource, SelectContainerConfigError, SelectContainerConfigRequest,
};
use crate::application::subagents::use_cases::SelectContainerConfig;

pub struct SelectedConfigLookup {
    selection: Arc<SelectContainerConfig>,
}

impl SelectedConfigLookup {
    pub fn new(selection: Arc<SelectContainerConfig>) -> Self {
        Self { selection }
    }
}

impl ContainerConfigLookup for SelectedConfigLookup {
    fn lookup(
        &self,
        target: &ContainerRuntimeTarget,
    ) -> Result<DiagnosableContainerConfig, String> {
        let selected = self
            .selection
            .execute(&SelectContainerConfigRequest {
                source: ContainerConfigSource::LaunchingAgent,
                name: target.name.clone(),
            })
            .map_err(|error| match error {
                // The launch's wording ("container: true refused … to
                // launch") is not the doctor's: name the command and
                // its own way out.
                SelectContainerConfigError::OverlayWithheld { diagnostics } => format!(
                    "container doctor refused: the checkout's repo-local config overlay was not applied, so the container config it labels default is unknown ({}); trust it (`quecto config trust`), or diagnose a global configuration's entry with --name",
                    diagnostics.join("; ")
                ),
                other => other.to_string(),
            })?;
        Ok(DiagnosableContainerConfig {
            name: selected.config.name,
            create: selected.config.create,
            inspect: selected.config.inspect,
            cleanup: selected.config.cleanup,
            diagnostics: selected.diagnostics,
        })
    }
}

impl std::fmt::Debug for SelectedConfigLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectedConfigLookup")
            .finish_non_exhaustive()
    }
}
