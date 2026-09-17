//! Resolve the effective configuration document of a run (#2024): the
//! base file (explicit or global) with the working directory's overlay
//! merged over it when — and only when — that overlay is trusted (recorded,
//! or consented to at the prompt an interactive adapter offers), carries no
//! global-only section, and is valid as a layer. An untrusted overlay is
//! reported, never applied; a trusted one that is broken is an error naming
//! the file, never a fallback; a merge that is invalid names both files.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::application::configuration::dto::{
    ConfigSelection, ConfigSources, EffectiveConfig, EffectiveConfigError, OverlayReport,
    OverlayState,
};
use crate::application::configuration::overlay_policy::{
    global_only_key, looks_like_config, merge_overlay,
};
use crate::application::configuration::ports::{
    ConfigDocumentStore, ConfigValidator, OverlayTrust, OverlayTrustStore,
};

pub struct ResolveEffectiveConfig {
    store: Arc<dyn ConfigDocumentStore>,
    validator: Arc<dyn ConfigValidator>,
    trust: Arc<dyn OverlayTrustStore>,
}

impl ResolveEffectiveConfig {
    pub fn new(
        store: Arc<dyn ConfigDocumentStore>,
        validator: Arc<dyn ConfigValidator>,
        trust: Arc<dyn OverlayTrustStore>,
    ) -> Self {
        Self {
            store,
            validator,
            trust,
        }
    }

    pub fn execute(
        &self,
        selection: &ConfigSelection,
    ) -> Result<EffectiveConfig, EffectiveConfigError> {
        match selection {
            ConfigSelection::Explicit(path) => {
                let Some(bytes) = self.read(path)? else {
                    return Err(EffectiveConfigError::Missing(path.clone()));
                };
                let document = self.resolved_object(path, &bytes)?;
                self.validate(path, &document)?;
                Ok(EffectiveConfig {
                    document: Value::Object(document),
                    sources: ConfigSources {
                        base: path.clone(),
                        explicit: true,
                        overlay: None,
                        legacy_local: None,
                    },
                })
            }
            ConfigSelection::Layered(layers) => {
                let global = match self.read(&layers.global)? {
                    Some(bytes) => self.resolved_object(&layers.global, &bytes)?,
                    None => Map::new(),
                };
                let (document, overlay) = match &layers.overlay {
                    Some(path) => self.apply_overlay(global, path)?,
                    None => (global, None),
                };
                match overlay
                    .as_ref()
                    .filter(|report| report.state == OverlayState::Applied)
                {
                    Some(applied) => self
                        .validator
                        .validate(&Value::Object(document.clone()))
                        .map_err(|reason| EffectiveConfigError::InvalidMerge {
                            global: layers.global.clone(),
                            overlay: applied.path.clone(),
                            reason,
                        })?,
                    None => self.validate(layers.global.as_path(), &document)?,
                }
                let legacy_local = layers
                    .legacy_local
                    .clone()
                    .filter(|path| self.is_retired_config(path));
                Ok(EffectiveConfig {
                    document: Value::Object(document),
                    sources: ConfigSources {
                        base: layers.global.clone(),
                        explicit: false,
                        overlay,
                        legacy_local,
                    },
                })
            }
        }
    }

    /// The global document with the overlay at `path` merged in when it is
    /// present, trusted and acceptable; otherwise the global document and
    /// the reason the overlay did not apply.
    fn apply_overlay(
        &self,
        global: Map<String, Value>,
        path: &Path,
    ) -> Result<(Map<String, Value>, Option<OverlayReport>), EffectiveConfigError> {
        let report = |state| {
            Some(OverlayReport {
                path: path.to_path_buf(),
                state,
            })
        };
        let Some(bytes) = self.read(path)? else {
            return Ok((global, report(OverlayState::Absent)));
        };
        let offered = match self.trust.decide(path, &bytes) {
            OverlayTrust::Trusted => false,
            OverlayTrust::Untrusted { fingerprint } => {
                if !self.trust.offer(path, &fingerprint) {
                    return Ok((global, report(OverlayState::Untrusted { fingerprint })));
                }
                true
            }
        };
        let overlay = self.resolved_object(path, &bytes)?;
        if let Some(key) = global_only_key(&overlay) {
            return Err(EffectiveConfigError::GlobalOnlyKey {
                path: path.to_path_buf(),
                key: key.to_string(),
            });
        }
        self.validator
            .validate_layer(&Value::Object(overlay.clone()))
            .map_err(|reason| EffectiveConfigError::Invalid {
                path: path.to_path_buf(),
                reason,
            })?;
        // Consent given at the prompt is recorded only now, after the same
        // checks `quecto config trust` applies; a failure above records
        // nothing.
        if offered {
            self.trust
                .approve(path, &bytes)
                .map_err(|reason| EffectiveConfigError::Read {
                    path: path.to_path_buf(),
                    reason: format!("could not record trust: {reason}"),
                })?;
        }
        Ok((
            merge_overlay(global, overlay),
            report(OverlayState::Applied),
        ))
    }

    /// A retired `<cwd>/config.json` is worth a warning only when it is a
    /// quecto configuration (a JSON object with a known section): many
    /// projects carry an unrelated `config.json` at their root.
    fn is_retired_config(&self, path: &Path) -> bool {
        self.store
            .read(path)
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|document| looks_like_config(&document))
    }

    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, EffectiveConfigError> {
        self.store
            .read(path)
            .map_err(|reason| EffectiveConfigError::Read {
                path: path.to_path_buf(),
                reason,
            })
    }

    /// Parse `bytes` as a JSON object and resolve its file-relative
    /// references against `path`.
    fn resolved_object(
        &self,
        path: &Path,
        bytes: &[u8],
    ) -> Result<Map<String, Value>, EffectiveConfigError> {
        let value: Value =
            serde_json::from_slice(bytes).map_err(|error| EffectiveConfigError::Parse {
                path: path.to_path_buf(),
                reason: error.to_string(),
            })?;
        if !value.is_object() {
            return Err(EffectiveConfigError::NotAnObject(path.to_path_buf()));
        }
        match self.validator.resolve(value, path) {
            Ok(Value::Object(object)) => Ok(object),
            Ok(_) => Err(EffectiveConfigError::NotAnObject(path.to_path_buf())),
            Err(reason) => Err(EffectiveConfigError::Invalid {
                path: path.to_path_buf(),
                reason,
            }),
        }
    }

    fn validate(
        &self,
        path: &Path,
        document: &Map<String, Value>,
    ) -> Result<(), EffectiveConfigError> {
        self.validator
            .validate(&Value::Object(document.clone()))
            .map_err(|reason| EffectiveConfigError::Invalid {
                path: path.to_path_buf(),
                reason,
            })
    }
}

impl std::fmt::Debug for ResolveEffectiveConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolveEffectiveConfig")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "resolve_effective_config_tests.rs"]
mod tests;
