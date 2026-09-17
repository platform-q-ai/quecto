//! Adapter of the subagent capability's [`EffectiveContainerConfigs`] port
//! (#2024 S4a) over the configuration capability's effective
//! configuration: the `container_configs` section of the `Config` a
//! source resolves to — the launching agent's layers (base file plus its
//! checkout's trusted overlay, entry-wise merged, untrusted or refused
//! overlay reported and not applied) or one explicit file, which replaces
//! the layers as `--config` does. Composition binds both loaders over the
//! configuration handles; this adapter only maps `Config` to the launch
//! vocabulary, so the one overlay and the one trust record
//! (`config-overlay-trust.json`, approved by `quecto config trust`) gate
//! container spawns exactly as they gate every other section.

use std::path::Path;
use std::sync::Arc;

use crate::application::subagents::dto::{
    ContainerConfigSource, ContainerConfigsError, ContainerLaunchConfig,
    EffectiveContainerConfigSet,
};
use crate::application::subagents::ports::EffectiveContainerConfigs;
use crate::infrastructure::config::Config;

/// A configuration resolved for a container launch: the realized `Config`
/// and the layer diagnostics the configuration capability reported.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub config: Config,
    pub diagnostics: Vec<String>,
}

/// The launching agent's effective configuration, bound by composition to
/// its selection (base file and checkout overlay); never prompts.
pub type LaunchingAgentConfigLoader = Arc<dyn Fn() -> Result<ResolvedConfig, String> + Send + Sync>;

/// One explicit file, loaded alone; the spawn call's `config` argument.
pub type ExplicitConfigLoader = Arc<dyn Fn(&Path) -> Result<ResolvedConfig, String> + Send + Sync>;

pub struct ContainerConfigsFromEffectiveConfig {
    launching_agent: Option<LaunchingAgentConfigLoader>,
    explicit: ExplicitConfigLoader,
}

impl ContainerConfigsFromEffectiveConfig {
    /// `launching_agent` is `None` for a launcher composed without an
    /// agent configuration (a rig with no parent config): a spawn that
    /// names no file then fails with the clear no-source error.
    pub fn new(
        launching_agent: Option<LaunchingAgentConfigLoader>,
        explicit: ExplicitConfigLoader,
    ) -> Self {
        Self {
            launching_agent,
            explicit,
        }
    }
}

impl EffectiveContainerConfigs for ContainerConfigsFromEffectiveConfig {
    fn effective_container_configs(
        &self,
        source: &ContainerConfigSource,
    ) -> Result<EffectiveContainerConfigSet, ContainerConfigsError> {
        let resolved = match source {
            ContainerConfigSource::LaunchingAgent => {
                self.launching_agent
                    .as_ref()
                    .ok_or(ContainerConfigsError::NoSource)?()
            }
            ContainerConfigSource::Explicit(path) => (self.explicit)(path),
        }
        .map_err(ContainerConfigsError::Invalid)?;
        Ok(EffectiveContainerConfigSet {
            configs: launch_configs(&resolved.config),
            diagnostics: resolved.diagnostics,
        })
    }
}

/// Every configured entry in the launch vocabulary, sorted by name.
pub fn launch_configs(config: &Config) -> Vec<ContainerLaunchConfig> {
    let mut configs: Vec<ContainerLaunchConfig> = config
        .container_configs
        .iter()
        .map(|(name, entry)| ContainerLaunchConfig {
            name: name.clone(),
            default: entry.default,
            create: entry.create.clone(),
            cleanup: entry.cleanup.clone(),
            exec: entry.exec.clone(),
            kill: entry.kill.clone(),
            inspect: entry.inspect.clone(),
        })
        .collect();
    configs.sort_by(|a, b| a.name.cmp(&b.name));
    configs
}

impl std::fmt::Debug for ContainerConfigsFromEffectiveConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerConfigsFromEffectiveConfig")
            .field("launching_agent", &self.launching_agent.is_some())
            .finish_non_exhaustive()
    }
}
