//! The catalogue's default-persistence ports (#2024 S2) over the one
//! configuration write path: `set_model … persist` and `set_effort …
//! persist` record `agents.defaults.model` / `agents.defaults.effort` in
//! the repository overlay or the global file through the configuration
//! capability's patch use case — the same exclusive hold, the same
//! validation of the layer and of the merge, trust recorded for exactly
//! the bytes written, `providers`/`admission` never touched — so a
//! default pinned from a running session and one pinned with `quecto
//! config set` are indistinguishable on disk.
//!
//! This adapter holds the composed patch handle and the run's selection;
//! it constructs neither (composition does). A run composed without a
//! reloadable configuration (rigs, the `models` CLI) gets the unbound
//! writer, which refuses every record naming why.

use std::sync::Arc;

use crate::application::catalogue::ports::{
    DefaultScope, EffortDefaultPersistence, ModelDefaultPersistence, PersistedDefault,
};
use crate::application::configuration::dto::{ConfigLayer, ConfigPatch, ConfigSelection};
use crate::application::configuration::use_cases::PatchConfiguration;
use crate::domain::provider::EffortLevel;

pub const MODEL_KEY_PATH: &str = "agents.defaults.model";
pub const EFFORT_KEY_PATH: &str = "agents.defaults.effort";

/// The patch handle bound to the selection of one run.
struct Bound {
    patch: Arc<PatchConfiguration>,
    selection: ConfigSelection,
}

pub struct ConfigDefaultsWriter {
    bound: Option<Bound>,
}

impl ConfigDefaultsWriter {
    /// Records defaults into the layers of `selection` through `patch`.
    pub fn new(patch: Arc<PatchConfiguration>, selection: ConfigSelection) -> Self {
        Self {
            bound: Some(Bound { patch, selection }),
        }
    }

    /// A writer for a run with no configuration to write into.
    pub fn unavailable() -> Self {
        Self { bound: None }
    }

    fn record(
        &self,
        scope: DefaultScope,
        key_path: &str,
        value: serde_json::Value,
    ) -> Result<PersistedDefault, String> {
        let Some(bound) = &self.bound else {
            return Err(
                "this run was started without a reloadable configuration, so there is no file to record a default in"
                    .to_string(),
            );
        };
        let layer = match scope {
            DefaultScope::Local => ConfigLayer::Overlay,
            DefaultScope::Global => ConfigLayer::Global,
        };
        let receipt = bound
            .patch
            .execute(ConfigPatch {
                selection: bound.selection.clone(),
                layer,
                key_path: key_path.to_string(),
                value,
            })
            .map_err(|error| error.to_string())?;
        Ok(PersistedDefault {
            scope,
            path: receipt.path,
        })
    }
}

impl ModelDefaultPersistence for ConfigDefaultsWriter {
    fn persist_model(&self, scope: DefaultScope, model: &str) -> Result<PersistedDefault, String> {
        self.record(
            scope,
            MODEL_KEY_PATH,
            serde_json::Value::String(model.to_string()),
        )
    }
}

impl EffortDefaultPersistence for ConfigDefaultsWriter {
    fn persist_effort(
        &self,
        scope: DefaultScope,
        level: EffortLevel,
    ) -> Result<PersistedDefault, String> {
        self.record(
            scope,
            EFFORT_KEY_PATH,
            serde_json::Value::String(level.as_str().to_string()),
        )
    }
}

impl std::fmt::Debug for ConfigDefaultsWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigDefaultsWriter")
            .field("bound", &self.bound.is_some())
            .finish()
    }
}

#[cfg(test)]
#[path = "defaults_tests.rs"]
mod tests;
