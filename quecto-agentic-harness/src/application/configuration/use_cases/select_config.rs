//! Select the configuration layers for a run (#1966, #2024).
//!
//! Precedence: an explicit override replaces everything; otherwise the
//! global file is the base and the working directory's
//! `.quecto/config.json` is the overlay candidate. Only the working
//! directory itself names an overlay — never its parents. Whether the
//! files exist, and whether the overlay is trusted, is decided when they
//! are loaded, not here.

use std::path::Path;

use crate::application::configuration::dto::config_selection::{
    LEGACY_LOCAL_FILE_NAME, OVERLAY_RELATIVE_PATH,
};
use crate::application::configuration::dto::{
    ConfigLayers, ConfigSelection, ConfigSelectionRequest,
};

#[derive(Debug, Default)]
pub struct SelectConfig;

impl SelectConfig {
    pub fn new() -> Self {
        Self
    }

    pub fn execute(&self, request: ConfigSelectionRequest) -> ConfigSelection {
        if let Some(explicit) = request.explicit {
            return ConfigSelection::Explicit(explicit);
        }
        let working_directory = request.working_directory.as_deref().map(Path::to_path_buf);
        ConfigSelection::Layered(ConfigLayers {
            global: request.global,
            overlay: working_directory
                .as_ref()
                .map(|cwd| cwd.join(OVERLAY_RELATIVE_PATH)),
            legacy_local: working_directory.map(|cwd| cwd.join(LEGACY_LOCAL_FILE_NAME)),
        })
    }
}

#[cfg(test)]
#[path = "select_config_tests.rs"]
mod tests;
