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
        let candidate = |relative: &str| {
            working_directory
                .as_ref()
                .map(|cwd| cwd.join(relative))
                .filter(|candidate| !same_file(candidate, &request.global))
        };
        ConfigSelection::Layered(ConfigLayers {
            overlay: candidate(OVERLAY_RELATIVE_PATH),
            legacy_local: candidate(LEGACY_LOCAL_FILE_NAME),
            global: request.global,
        })
    }
}

/// Lexical identity after normalising `.` and `..` components: the global
/// file is never its own overlay or its own retired local file.
fn same_file(a: &Path, b: &Path) -> bool {
    normalise(a) == normalise(b)
}

fn normalise(path: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
#[path = "select_config_tests.rs"]
mod tests;
