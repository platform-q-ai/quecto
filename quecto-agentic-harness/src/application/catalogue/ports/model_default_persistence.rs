//! Persist the model a switch resolved as a configured default (#2024 S2):
//! `agents.defaults.model` in the repository overlay or the global file.
//! The change-active-model use case is its only caller; composition maps
//! it onto the configuration capability's patch use case
//! (`composition/catalogue_defaults.rs`), so the write carries that use
//! case's guarantees (exclusive hold, validation, trust recorded for the
//! bytes written, global-only sections untouched).

pub use crate::application::catalogue::dto::{DefaultScope, PersistedDefault};

pub trait ModelDefaultPersistence: Send + Sync {
    /// Record `model` (a qualified `provider/model` id) as the default of
    /// `scope`. Nothing is written when the adapter refuses; the reason
    /// names the remedy.
    fn persist_model(&self, scope: DefaultScope, model: &str) -> Result<PersistedDefault, String>;
}

/// A recording fake of both default-persistence ports for use-case tests
/// and rigs: every accepted record is kept, and `refusing` makes every
/// record fail with that reason.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default)]
pub struct RecordedDefaults {
    pub records: std::sync::Mutex<Vec<(DefaultScope, String, String)>>,
    pub refusal: Option<String>,
}

#[cfg(any(test, feature = "test-support"))]
impl RecordedDefaults {
    pub fn refusing(reason: &str) -> Self {
        Self {
            records: std::sync::Mutex::new(Vec::new()),
            refusal: Some(reason.to_string()),
        }
    }

    fn record(
        &self,
        scope: DefaultScope,
        key: &str,
        value: &str,
    ) -> Result<PersistedDefault, String> {
        if let Some(reason) = &self.refusal {
            return Err(reason.clone());
        }
        self.records
            .lock()
            .unwrap()
            .push((scope, key.to_string(), value.to_string()));
        Ok(PersistedDefault {
            scope,
            path: std::path::PathBuf::from(format!("/fake/{}.json", scope.as_str())),
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ModelDefaultPersistence for RecordedDefaults {
    fn persist_model(&self, scope: DefaultScope, model: &str) -> Result<PersistedDefault, String> {
        self.record(scope, "agents.defaults.model", model)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl super::EffortDefaultPersistence for RecordedDefaults {
    fn persist_effort(
        &self,
        scope: DefaultScope,
        level: crate::domain::provider::EffortLevel,
    ) -> Result<PersistedDefault, String> {
        self.record(scope, "agents.defaults.effort", level.as_str())
    }
}
