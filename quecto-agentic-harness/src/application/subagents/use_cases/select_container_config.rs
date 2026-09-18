//! Container config selection at launch (#2024 S4a): which named
//! `container_configs` entry a new container is created with. Launch
//! policy, not file plumbing: the entries come from the configuration
//! capability's effective configuration for the launching agent's checkout
//! (or an explicit file the spawn call named) through the
//! [`EffectiveContainerConfigs`] port; this use case owns the rules —
//! an explicit name wins, otherwise the one entry labelled `"default":
//! true`; an implicit default is refused while the checkout's overlay is
//! withheld — not applied and able to have changed the default (refused,
//! unparseable, or declaring `container_configs`) — because the default
//! it labels is unknown; an explicit name launches from the global set and carries
//! the overlay's diagnostic; every refusal enumerates the live names so
//! an agent can offer the menu, and an unknown name or missing default
//! carries the layer diagnostics too (a withheld overlay is the likely
//! reason the entry is absent); a selected entry must carry runnable
//! `create` and `cleanup` argv with no empty or NUL-bearing argument.

use std::sync::Arc;

use crate::application::subagents::dto::{
    ContainerConfigSource, ContainerLaunchConfig, SelectContainerConfigError,
    SelectContainerConfigRequest, SelectedContainerConfig,
};
use crate::application::subagents::ports::EffectiveContainerConfigs;

pub struct SelectContainerConfig {
    configs: Arc<dyn EffectiveContainerConfigs>,
}

impl SelectContainerConfig {
    pub fn new(configs: Arc<dyn EffectiveContainerConfigs>) -> Self {
        Self { configs }
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
            None => {
                // The configuration capability enforces exactly one default
                // for a non-empty set, so the only way to arrive here without
                // one is an empty set; the arm still enumerates so a bypass
                // cannot fail silently.
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
        };
        validate_argv(config)?;
        Ok(SelectedContainerConfig {
            config: config.clone(),
            diagnostics: set.diagnostics,
        })
    }
}

fn validate_argv(config: &ContainerLaunchConfig) -> Result<(), SelectContainerConfigError> {
    let invalid = |what| SelectContainerConfigError::InvalidArgv {
        name: config.name.clone(),
        what,
    };
    if config.create.is_empty() {
        return Err(invalid("missing create argv"));
    }
    if config.cleanup.is_empty() {
        return Err(invalid("missing cleanup argv"));
    }
    let unsafe_arg = |arg: &String| arg.is_empty() || arg.contains('\0');
    if config
        .create
        .iter()
        .chain(&config.cleanup)
        .chain(&config.exec)
        .chain(&config.kill)
        .chain(&config.inspect)
        .any(unsafe_arg)
    {
        return Err(invalid("unsafe argv"));
    }
    Ok(())
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
