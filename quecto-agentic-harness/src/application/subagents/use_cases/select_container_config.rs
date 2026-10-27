//! Container config selection at launch (#2024 S4a): which named
//! `container_configs` entry a new container is created with. Launch
//! policy, not file plumbing: the entries come from the configuration
//! capability's effective configuration for the launching agent's checkout
//! (or an explicit file the spawn call named) through the
//! [`EffectiveContainerConfigs`] port; this use case owns the rules —
//! an explicit name wins; otherwise the checkout's own `standard` entry
//! (the one `quecto container init` writes into its overlay, #2035) is
//! the repo's default whatever entry the global file or another overlay
//! entry labels `"default": true` — a rule here, not a label, so a
//! hand-edited file cannot flip it; otherwise the one entry labelled
//! `"default": true`; an implicit default is refused while the checkout's overlay is
//! withheld — not applied and able to have changed the default (refused,
//! unparseable, or declaring `container_configs`) — because the default
//! it labels is unknown; an explicit name launches from the global set and carries
//! the overlay's diagnostic; every refusal enumerates the live names so
//! an agent can offer the menu, and an unknown name or missing default
//! carries the layer diagnostics too (a withheld overlay is the likely
//! reason the entry is absent); a selected entry must carry runnable
//! `create` and `cleanup` argv with no empty or NUL-bearing argument;
//! and every script of the selected entry that is a standard-bundle
//! asset (#2024 S4e) must still carry the bytes this binary embeds —
//! those scripts run on the host before any container exists, and the
//! overlay's trust covers the entry, not the files it names — else the
//! launch is refused naming the file and the refresh, through the
//! [`ContainerScriptIntegrity`] port.
//!
//! A swarm's selection ([`SelectContainerConfig::select_for_swarm`]) is
//! the same selection, then confirmed to clone the launching project's own
//! repository ([`ConfirmContainerRepository`], swarm-container-matches-repo):
//! a global default names one repository and serves only that one. A
//! selection composed without the confirmation refuses every swarm.

use std::sync::Arc;

use crate::application::subagents::dto::{
    container_config::{
        ContainerConfigSource, ContainerLaunchConfig, SelectContainerConfigError,
        SelectContainerConfigRequest, SelectedContainerConfig,
    },
    standard_script::StandardScriptVerdict,
};
use crate::application::subagents::ports::container_configs::{
    ContainerScriptIntegrity, EffectiveContainerConfigs,
};
use crate::application::subagents::use_cases::ConfirmContainerRepository;

pub struct SelectContainerConfig {
    configs: Arc<dyn EffectiveContainerConfigs>,
    integrity: Arc<dyn ContainerScriptIntegrity>,
    /// A swarm's repository check; `None` refuses every swarm.
    repository: Option<Arc<ConfirmContainerRepository>>,
}

impl SelectContainerConfig {
    pub fn new(
        configs: Arc<dyn EffectiveContainerConfigs>,
        integrity: Arc<dyn ContainerScriptIntegrity>,
    ) -> Self {
        Self {
            configs,
            integrity,
            repository: None,
        }
    }

    /// This selection, confirming a swarm's container with `repository`.
    #[must_use]
    pub fn confirming(self, repository: Arc<ConfirmContainerRepository>) -> Self {
        Self {
            repository: Some(repository),
            ..self
        }
    }

    /// The config a swarm's new container launches with: [`Self::execute`]'s
    /// selection, confirmed to clone the launching project's own
    /// repository. Nothing is created by the selection or the check.
    ///
    /// # Errors
    /// [`Self::execute`]'s; the repository check's refusal; no check
    /// composed.
    pub fn select_for_swarm(
        &self,
        request: &SelectContainerConfigRequest,
    ) -> Result<SelectedContainerConfig, SelectContainerConfigError> {
        let selected = self.execute(request)?;
        let Some(repository) = self.repository.as_deref() else {
            return Err(SelectContainerConfigError::NoRepositoryCheck {
                name: selected.config.name,
            });
        };
        let confirmed = repository
            .confirm(&selected.config)
            .map_err(SelectContainerConfigError::Repository)?;
        debug_assert!(
            selected.config.repository.is_some(),
            "a confirmed container names the repository {confirmed}"
        );
        Ok(selected)
    }

    pub fn execute(
        &self,
        request: &SelectContainerConfigRequest,
    ) -> Result<SelectedContainerConfig, SelectContainerConfigError> {
        if let ContainerConfigSource::Explicit(path) = &request.source
            && !path.is_absolute()
        {
            return Err(SelectContainerConfigError::RelativeConfigPath(path.clone()));
        }
        let set = self
            .configs
            .effective_container_configs(&request.source)
            .map_err(SelectContainerConfigError::Unavailable)?;
        let available = set.names();
        if request.name.is_none() && set.overlay_withheld {
            return Err(SelectContainerConfigError::OverlayWithheld {
                diagnostics: set.diagnostics,
            });
        }
        let config = match request.name.as_deref() {
            Some(name) => set
                .configs
                .iter()
                .find(|config| config.name == name)
                .ok_or_else(|| SelectContainerConfigError::Unknown {
                    name: name.to_string(),
                    available: available.clone(),
                    diagnostics: set.diagnostics.clone(),
                })?,
            // The checkout's own standard entry is its default by rule
            // (#2035): no label anywhere is consulted while one exists.
            None => match set.repo_standard() {
                Some(standard) => standard,
                None => {
                    // The configuration capability enforces exactly one
                    // default for a non-empty loaded set, so the only way
                    // to arrive here without one is an empty set; the arm
                    // still enumerates so a bypass (an in-memory set no
                    // loader would produce, as the tests build) cannot
                    // fail silently.
                    let mut defaults: Vec<&ContainerLaunchConfig> =
                        set.configs.iter().filter(|config| config.default).collect();
                    defaults.sort_by(|a, b| a.name.cmp(&b.name));
                    match defaults.as_slice() {
                        [only] => *only,
                        _ => {
                            return Err(SelectContainerConfigError::NoDefault {
                                available,
                                diagnostics: set.diagnostics.clone(),
                            });
                        }
                    }
                }
            },
        };
        validate_argv(config)?;
        self.verify_standard_scripts(config)?;
        Ok(SelectedContainerConfig {
            config: config.clone(),
            diagnostics: set.diagnostics,
        })
    }
}

impl SelectContainerConfig {
    /// Every script the entry names that belongs to the standard bundle
    /// must be intact; the first that is not names the refusal.
    fn verify_standard_scripts(
        &self,
        config: &ContainerLaunchConfig,
    ) -> Result<(), SelectContainerConfigError> {
        for script in config.scripts() {
            match self.integrity.verify(&script) {
                StandardScriptVerdict::NotStandard | StandardScriptVerdict::Intact => {}
                verdict @ (StandardScriptVerdict::Differs
                | StandardScriptVerdict::Outdated
                | StandardScriptVerdict::Missing
                | StandardScriptVerdict::Refused(_)) => {
                    return Err(SelectContainerConfigError::StandardScriptAltered {
                        name: config.name.clone(),
                        script,
                        verdict,
                    });
                }
            }
        }
        Ok(())
    }
}

fn validate_argv(config: &ContainerLaunchConfig) -> Result<(), SelectContainerConfigError> {
    match config.argv_problem() {
        Some(what) => Err(SelectContainerConfigError::InvalidArgv {
            name: config.name.clone(),
            what,
        }),
        None => Ok(()),
    }
}

impl std::fmt::Debug for SelectContainerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectContainerConfig")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "select_container_config_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "select_container_config_swarm_tests.rs"]
mod swarm_tests;
