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
) -> Result<LoadedConfig, String> {
    let handles = build_configuration(&ConfigurationEnvironment {
        base_dir: base_dir.to_path_buf(),
        prompt_for_trust,
    });
    let effective = handles
        .resolve
        .execute(selection)
        .map_err(|error| error.to_string())?;
    let config = crate::infrastructure::config::mapping::realize_config(
        effective.document,
        env_overrides,
        base_dir,
    )?;
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

/// What the user should know about the layers, one line each: an overlay
/// that was present but not applied, and a retired working-directory file.
pub fn layer_diagnostics(sources: &ConfigSources) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(overlay) = &sources.overlay
        && let OverlayState::Untrusted { fingerprint } = &overlay.state
    {
        lines.push(format!(
            "repo-local config overlay {} is not trusted (sha256 {fingerprint}) and was not applied; review it, then run `quecto config trust` from this directory",
            overlay.path.display()
        ));
    }
    if let Some(legacy) = &sources.legacy_local {
        lines.push(format!(
            "warning: {} is no longer loaded (a working-directory config.json used to replace the global file); move its repo-specific settings to {} with `quecto config set`, and its providers or admission section to the global file",
            legacy.display(),
            legacy
                .parent()
                .unwrap_or(legacy)
                .join(".quecto/config.json")
                .display()
        ));
    }
    lines
}

/// The `Overlay:` line of `quecto status`.
pub(crate) fn overlay_summary(sources: &ConfigSources) -> String {
    match &sources.overlay {
        Some(report) => match &report.state {
            OverlayState::Applied => format!("{} (trusted)", report.path.display()),
            OverlayState::Untrusted { .. } => format!("{} (untrusted)", report.path.display()),
            OverlayState::Absent => "none".to_string(),
        },
        None if sources.explicit => "none (--config replaces both layers)".to_string(),
        None => "none".to_string(),
    }
}

#[cfg(test)]
#[path = "config_loading_tests.rs"]
mod tests;
