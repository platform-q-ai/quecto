//! `quecto container doctor` (#2024 S4b): the create preflight of the
//! effective container config, run without creating an environment. The
//! use case resolves the target through the lookup port, asks the
//! preflight port for the script's checks, and assembles the diagnosis;
//! the checks themselves are the script's (one list serves the create and
//! the doctor), so nothing here knows a runtime by name. A config whose
//! script cannot answer — missing, refusing `--preflight-only`, silent —
//! is an error naming the config, never an empty, healthy-looking report.

use std::sync::Arc;

use crate::application::environments::dto::{
    ContainerRuntimeDiagnosis, ContainerRuntimeTarget, DiagnoseContainerRuntimeError,
};
use crate::application::environments::ports::{ContainerConfigLookup, ContainerRuntimePreflight};

pub struct DiagnoseContainerRuntime {
    lookup: Arc<dyn ContainerConfigLookup>,
    preflight: Arc<dyn ContainerRuntimePreflight>,
}

impl DiagnoseContainerRuntime {
    pub fn new(
        lookup: Arc<dyn ContainerConfigLookup>,
        preflight: Arc<dyn ContainerRuntimePreflight>,
    ) -> Self {
        Self { lookup, preflight }
    }

    pub fn execute(
        &self,
        target: &ContainerRuntimeTarget,
    ) -> Result<ContainerRuntimeDiagnosis, DiagnoseContainerRuntimeError> {
        let config = self
            .lookup
            .lookup(target)
            .map_err(DiagnoseContainerRuntimeError::ConfigUnavailable)?;
        if config.create.is_empty() {
            return Err(DiagnoseContainerRuntimeError::PreflightUnavailable {
                config: config.name,
                detail: "the config has no create argv to run a preflight with".into(),
            });
        }
        let checks = self.preflight.preflight(&config).map_err(|detail| {
            DiagnoseContainerRuntimeError::PreflightUnavailable {
                config: config.name.clone(),
                detail,
            }
        })?;
        if checks.is_empty() {
            return Err(DiagnoseContainerRuntimeError::PreflightUnavailable {
                config: config.name,
                detail: format!(
                    "create script `{}` reported no preflight checks",
                    crate::domain::redaction::redact_url_userinfo(&config.create.join(" "))
                ),
            });
        }
        Ok(ContainerRuntimeDiagnosis {
            config: config.name,
            create: config.create,
            checks,
            diagnostics: config.diagnostics,
        })
    }
}

impl std::fmt::Debug for DiagnoseContainerRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiagnoseContainerRuntime")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "diagnose_container_runtime_tests.rs"]
mod tests;
