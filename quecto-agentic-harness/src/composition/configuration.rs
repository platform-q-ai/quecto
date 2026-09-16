//! Configuration composition (#1966): the selection use case over the
//! filesystem probe. `main` hands [`build_select_config`] to the CLI entry
//! point; the interface only invokes the handle it receives.

use std::sync::Arc;

use crate::application::configuration::use_cases::SelectConfig;
use crate::infrastructure::local_config_probe::FilesystemLocalConfigProbe;

pub fn build_select_config() -> Arc<SelectConfig> {
    Arc::new(SelectConfig::new(Arc::new(FilesystemLocalConfigProbe)))
}

#[cfg(test)]
#[path = "configuration_tests.rs"]
mod tests;
