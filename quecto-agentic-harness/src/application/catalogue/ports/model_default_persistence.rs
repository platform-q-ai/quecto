//! Persist the model a switch resolved as a configured default (#2024 S2):
//! `agents.defaults.model` in the repository overlay or the global file.
//! The change-active-model use case is its only caller; infrastructure
//! implements it over the one safe configuration writer, so the write
//! carries that writer's guarantees (exclusive hold, validation, trust
//! recorded for the bytes written, global-only sections untouched).

use std::path::PathBuf;

/// Which configuration layer a default is persisted into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultScope {
    /// The repository overlay (`<cwd>/.quecto/config.json`).
    Local,
    /// The global file (`<base_dir>/config.json`).
    Global,
}

impl DefaultScope {
    /// The wire and CLI spelling of the scope.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Global => "global",
        }
    }
}

/// Where a default landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedDefault {
    pub scope: DefaultScope,
    pub path: PathBuf,
}

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
            path: PathBuf::from(format!("/fake/{}.json", scope.as_str())),
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
