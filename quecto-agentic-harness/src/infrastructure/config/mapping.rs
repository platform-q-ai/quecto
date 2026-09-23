//! Document ↔ `Config` mapping (#2024): the JSON document a run resolved
//! (the global file with a trusted overlay merged in, or a document the
//! safe writer is about to persist) realized into the `Config` struct with
//! every load-time validation, the environment overrides, and the
//! [`ConfigValidator`] port the configuration use cases validate through.
//! The section-wise overlay merge itself is a pure function of the
//! application (`application::configuration::overlay_policy`), reached by
//! the resolve use case directly.

use std::collections::HashMap;
use std::path::Path;

use crate::application::configuration::ports::ConfigValidator;
use crate::infrastructure::config::{Config, ConfigError};

impl Config {
    /// Reject the keys the schema retired, before anything is deserialized:
    /// an honest breaking-window signal, not a compat shim — the pre-#1410
    /// key would otherwise be silently ignored and containers would quietly
    /// become "none configured".
    pub fn reject_retired_keys(value: &serde_json::Value) -> Result<(), ConfigError> {
        if value.get("container_scripts").is_some() {
            return Err(ConfigError::ContainerConfigs(
                "the `container_scripts` key was renamed to `container_configs` (#1410); \
                 entries are now a flat map of container configs with exactly one labeled \
                 \"default\": true — see docs/container-runtimes.md"
                    .into(),
            ));
        }
        Ok(())
    }

    /// Resolve a document's file-relative references (workflow step `ref`s
    /// against the directory of `path`) so it can be merged with documents
    /// that lived elsewhere (#2024). Retired keys are rejected first.
    pub fn resolve_document(
        mut value: serde_json::Value,
        path: &Path,
    ) -> Result<serde_json::Value, ConfigError> {
        Self::reject_retired_keys(&value)?;
        super::resolve_workflow_step_entries(&mut value, path)?;
        Ok(value)
    }

    /// A configuration from an already-resolved JSON document (#2024): the
    /// effective merge of the global file and a repo-local overlay, or any
    /// document the safe writer is about to persist. Every load-time
    /// validation applies.
    pub fn from_inherited_child_document(value: serde_json::Value) -> Result<Self, ConfigError> {
        let config: Config = serde_json::from_value(value).map_err(ConfigError::Parse)?;
        config.validate_effort()?;
        config.validate_container_configs()?;
        Ok(config)
    }

    pub fn from_document(value: serde_json::Value) -> Result<Self, ConfigError> {
        let config: Config = serde_json::from_value(value).map_err(ConfigError::Parse)?;
        config.validated()
    }

    /// One layer of a layered configuration (#2024): the schema and every
    /// per-field rule apply, but "exactly one default container config"
    /// only holds for the complete configuration, so a layer adding a
    /// non-default entry is valid and the merge is checked in full.
    pub fn layer_from_document(value: serde_json::Value) -> Result<Self, ConfigError> {
        let config: Config = serde_json::from_value(value).map_err(ConfigError::Parse)?;
        config.validate_effort()?;
        config.validate_admission()?;
        Ok(config)
    }

    pub(super) fn validated(self) -> Result<Self, ConfigError> {
        self.validate_effort()?;
        self.validate_container_configs()?;
        self.validate_admission()?;
        Ok(self)
    }

    /// Apply environment variable overrides to a loaded configuration and
    /// re-validate (an unknown `QUECTO_AGENTS_DEFAULTS_EFFORT` is rejected,
    /// #1066). Overrides may carry secrets and are never written back.
    pub fn with_env_overrides(
        mut self,
        env_overrides: &HashMap<String, String>,
    ) -> Result<Self, ConfigError> {
        Self::apply_env_overrides(&mut self, env_overrides);
        self.validate_effort()?;
        self.validate_container_configs()?;
        Ok(self)
    }
}

/// The `Config` struct as the [`ConfigValidator`] port.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConfigValidatorAdapter;

impl ConfigValidator for ConfigValidatorAdapter {
    fn resolve(
        &self,
        document: serde_json::Value,
        path: &Path,
    ) -> Result<serde_json::Value, String> {
        Config::resolve_document(document, path).map_err(|error| error.to_string())
    }

    fn validate(&self, document: &serde_json::Value) -> Result<(), String> {
        Config::from_document(document.clone())
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn validate_inherited_child(&self, document: &serde_json::Value) -> Result<(), String> {
        Config::from_inherited_child_document(document.clone())
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn validate_layer(&self, document: &serde_json::Value) -> Result<(), String> {
        Config::layer_from_document(document.clone())
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

/// The `Config` a run operates on, from the effective document the
/// application resolved: deserialized and validated, then the environment
/// overrides applied (they may carry secrets and are never written back),
/// then bound to the base directory the admission default directory
/// derives from.
pub fn realize_config(
    document: serde_json::Value,
    env_overrides: &HashMap<String, String>,
    base_dir: &Path,
) -> Result<Config, String> {
    Config::from_document(document)
        .and_then(|config| config.with_env_overrides(env_overrides))
        .map(|config| config.with_admission_base_dir(base_dir))
        .map_err(|error| error.to_string())
}

pub fn realize_inherited_child_config(
    document: serde_json::Value,
    env_overrides: &HashMap<String, String>,
    base_dir: &Path,
) -> Result<Config, String> {
    Config::from_inherited_child_document(document)
        .and_then(|config| config.with_env_overrides(env_overrides))
        .map(|config| config.with_admission_base_dir(base_dir))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "mapping_tests.rs"]
mod tests;
