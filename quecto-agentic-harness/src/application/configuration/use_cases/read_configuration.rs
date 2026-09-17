//! Read configuration values (#2024): one layer's document as written, or
//! the effective merge a run would load, narrowed to a dotted key path.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::application::configuration::dto::{
    ConfigReadError, ConfigReadRequest, ConfigReadScope, ConfigReadout,
};
use crate::application::configuration::overlay_policy::get_path;
use crate::application::configuration::ports::ConfigDocumentStore;
use crate::application::configuration::use_cases::ResolveEffectiveConfig;

pub struct ReadConfiguration {
    store: Arc<dyn ConfigDocumentStore>,
    resolve: Arc<ResolveEffectiveConfig>,
}

impl ReadConfiguration {
    pub fn new(store: Arc<dyn ConfigDocumentStore>, resolve: Arc<ResolveEffectiveConfig>) -> Self {
        Self { store, resolve }
    }

    pub fn execute(&self, request: ConfigReadRequest) -> Result<ConfigReadout, ConfigReadError> {
        let (document, sources) = match request.scope {
            ConfigReadScope::Effective => {
                let effective = self
                    .resolve
                    .execute(&request.selection)
                    .map_err(ConfigReadError::Effective)?;
                (effective.document, Some(effective.sources))
            }
            ConfigReadScope::Global => (self.raw(request.selection.path())?, None),
            ConfigReadScope::Overlay => {
                let path = request
                    .selection
                    .overlay_path()
                    .ok_or(ConfigReadError::NoOverlayLocation)?;
                (self.raw(path)?, None)
            }
        };
        let value = match &request.key_path {
            Some(key_path) => get_path(&document, key_path)
                .cloned()
                .ok_or_else(|| ConfigReadError::NotSet(key_path.clone()))?,
            None => document,
        };
        Ok(ConfigReadout { value, sources })
    }

    /// The file as written; an absent file reads as an empty object.
    fn raw(&self, path: &Path) -> Result<Value, ConfigReadError> {
        let bytes = self
            .store
            .read(path)
            .map_err(|reason| ConfigReadError::Read {
                path: path.to_path_buf(),
                reason,
            })?;
        match bytes {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(|error| ConfigReadError::Parse {
                path: path.to_path_buf(),
                reason: error.to_string(),
            }),
            None => Ok(Value::Object(serde_json::Map::new())),
        }
    }
}

impl std::fmt::Debug for ReadConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadConfiguration").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "read_configuration_tests.rs"]
mod tests;
