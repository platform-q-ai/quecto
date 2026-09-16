//! Select the configuration file for a run (#1966).
//!
//! Precedence: an explicit override, then `config.json` in the working
//! directory, then the global file. Only the working directory itself is
//! probed — never its parents — and a local file that is present but not a
//! usable regular file is an error, not a fallback.

use std::path::Path;
use std::sync::Arc;

use crate::application::configuration::dto::{
    ConfigSelection, ConfigSelectionError, ConfigSelectionRequest, LocalConfigRejection,
};
use crate::application::configuration::ports::{LocalConfigPresence, LocalConfigProbe};

/// File name discovered in the working directory.
pub const LOCAL_CONFIG_FILE_NAME: &str = "config.json";

pub struct SelectConfig {
    probe: Arc<dyn LocalConfigProbe>,
}

impl SelectConfig {
    pub fn new(probe: Arc<dyn LocalConfigProbe>) -> Self {
        Self { probe }
    }

    pub fn execute(
        &self,
        request: ConfigSelectionRequest,
    ) -> Result<ConfigSelection, ConfigSelectionError> {
        if let Some(explicit) = request.explicit {
            return Ok(ConfigSelection::Explicit(explicit));
        }
        let Some(working_directory) = request.working_directory else {
            return Ok(ConfigSelection::Global(request.global));
        };
        let local = Path::new(&working_directory).join(LOCAL_CONFIG_FILE_NAME);
        match self.probe.probe(&local) {
            LocalConfigPresence::Absent => Ok(ConfigSelection::Global(request.global)),
            LocalConfigPresence::RegularFile => Ok(ConfigSelection::WorkingDirectory(local)),
            LocalConfigPresence::NotRegularFile => Err(ConfigSelectionError {
                path: local,
                rejection: LocalConfigRejection::NotRegularFile,
            }),
            LocalConfigPresence::Unreadable(reason) => Err(ConfigSelectionError {
                path: local,
                rejection: LocalConfigRejection::Unreadable(reason),
            }),
        }
    }
}

impl std::fmt::Debug for SelectConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectConfig").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "select_config_tests.rs"]
mod tests;
