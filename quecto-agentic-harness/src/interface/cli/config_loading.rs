//! Loading the run's configuration through the configuration capability
//! (#2024): the selected layers resolved into one effective document, then
//! realized into the `Config` the rest of the CLI operates on, plus the
//! diagnostics a user should see about the layers (an untrusted overlay,
//! a retired `./config.json`).

use std::collections::HashMap;
use std::path::Path;

use super::ConfigurationHandlesBuilder;
use super::configuration_handles::ConfigurationEnvironment;
use crate::application::configuration::dto::{ConfigSelection, ConfigSources, OverlayState};
use crate::infrastructure::config::Config;

pub(crate) struct LoadedConfig {
    pub config: Config,
    pub sources: ConfigSources,
}

/// Resolve and realize the configuration for `selection`. An explicit
/// file that is missing, an unreadable or invalid layer, or an overlay
/// carrying a global-only section is an error naming the file.
pub(crate) fn load_selected_config(
    build_configuration: ConfigurationHandlesBuilder,
    base_dir: &Path,
    selection: &ConfigSelection,
    prompt_for_trust: bool,
    env_overrides: &HashMap<String, String>,
    inherited_child: bool,
) -> Result<LoadedConfig, String> {
    let handles = build_configuration(&ConfigurationEnvironment {
        base_dir: base_dir.to_path_buf(),
        prompt_for_trust,
    });
    let effective = (if inherited_child {
        handles.resolve.execute_inherited_child(selection)
    } else {
        handles.resolve.execute(selection)
    })
    .map_err(|error| error.to_string())?;
    let config = (if inherited_child {
        &handles.realize_inherited_child
    } else {
        &handles.realize
    })(effective.document, env_overrides)?;
    Ok(LoadedConfig {
        config,
        sources: effective.sources,
    })
}

/// The `QUECTO_*` environment of this process, the overrides every agent
/// run applies over the files.
pub(crate) fn quecto_env_overrides() -> HashMap<String, String> {
    std::env::vars()
        .filter(|(key, _)| key.starts_with("QUECTO_"))
        .collect()
}

/// The layer diagnostics, as the DTO renders them.
pub(crate) fn layer_diagnostics(sources: &ConfigSources) -> Vec<String> {
    sources.diagnostics()
}

/// The `Overlay:` line of `quecto status`.
pub(crate) fn overlay_summary(sources: &ConfigSources) -> String {
    match &sources.overlay {
        Some(report) => match &report.state {
            OverlayState::Applied => format!("{} (trusted)", report.path.display()),
            OverlayState::Untrusted { .. } => format!("{} (untrusted)", report.path.display()),
            OverlayState::Refused { .. } => format!("{} (refused)", report.path.display()),
            OverlayState::Absent => "none".to_string(),
        },
        None if sources.explicit => "none (--config replaces both layers)".to_string(),
        None => "none".to_string(),
    }
}

#[cfg(test)]
#[path = "config_loading_tests.rs"]
mod tests;
