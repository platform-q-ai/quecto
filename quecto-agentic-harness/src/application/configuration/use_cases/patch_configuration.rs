//! Patch one key of one configuration document (#2024): the safe writer
//! every configuration change goes through.
//!
//! Guarantees: the document is patched as a JSON document (never through
//! the validator's struct, so unknown keys and key order survive), only the
//! addressed path changes, the result must pass validator validation before a
//! byte is written, and the write is all-or-nothing through the store. An
//! overlay patch additionally refuses global-only keys, refuses to touch an
//! overlay whose current content is not trusted, and re-records trust for
//! the content it wrote.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::application::configuration::dto::{
    ConfigLayer, ConfigPatch, ConfigPatchError, ConfigPatchReceipt,
};
use crate::application::configuration::overlay_policy::{GLOBAL_ONLY_KEYS, key_segments, set_path};
use crate::application::configuration::ports::{
    ConfigDocumentStore, ConfigDocumentWriter, ConfigValidator, OverlayTrust, OverlayTrustStore,
};

pub struct PatchConfiguration {
    store: Arc<dyn ConfigDocumentStore>,
    writer: Arc<dyn ConfigDocumentWriter>,
    validator: Arc<dyn ConfigValidator>,
    trust: Arc<dyn OverlayTrustStore>,
}

impl PatchConfiguration {
    pub fn new(
        store: Arc<dyn ConfigDocumentStore>,
        writer: Arc<dyn ConfigDocumentWriter>,
        validator: Arc<dyn ConfigValidator>,
        trust: Arc<dyn OverlayTrustStore>,
    ) -> Self {
        Self {
            store,
            writer,
            validator,
            trust,
        }
    }

    pub fn execute(&self, patch: ConfigPatch) -> Result<ConfigPatchReceipt, ConfigPatchError> {
        let path = patch.target.path.as_path();
        let segments = key_segments(&patch.key_path)
            .ok_or_else(|| ConfigPatchError::InvalidKeyPath(patch.key_path.clone()))?;
        if patch.target.layer == ConfigLayer::Overlay && GLOBAL_ONLY_KEYS.contains(&segments[0]) {
            return Err(ConfigPatchError::GlobalOnlyKey {
                path: path.to_path_buf(),
                key: segments[0].to_string(),
            });
        }
        let existing = self
            .store
            .read(path)
            .map_err(|reason| ConfigPatchError::Read {
                path: path.to_path_buf(),
                reason,
            })?;
        if let (ConfigLayer::Overlay, Some(bytes)) = (patch.target.layer, &existing)
            && let OverlayTrust::Untrusted { fingerprint } = self.trust.decide(path, bytes)
        {
            return Err(ConfigPatchError::UntrustedOverlay {
                path: path.to_path_buf(),
                fingerprint,
            });
        }
        let mut document = match &existing {
            Some(bytes) => {
                serde_json::from_slice(bytes).map_err(|error| ConfigPatchError::Parse {
                    path: path.to_path_buf(),
                    reason: error.to_string(),
                })?
            }
            None => Value::Object(serde_json::Map::new()),
        };
        set_path(&mut document, &patch.key_path, patch.value).map_err(|at| {
            ConfigPatchError::NotAnObject {
                path: path.to_path_buf(),
                at,
            }
        })?;
        self.check(path, &document)?;
        self.writer
            .write(path, &document)
            .map_err(|reason| ConfigPatchError::Write {
                path: path.to_path_buf(),
                reason,
            })?;
        if patch.target.layer == ConfigLayer::Overlay {
            // Trust is by exact bytes, so the approval reads back what the
            // writer laid down rather than guessing its rendering.
            let written = self
                .store
                .read(path)
                .map_err(|reason| ConfigPatchError::Trust {
                    path: path.to_path_buf(),
                    reason,
                })?
                .ok_or_else(|| ConfigPatchError::Trust {
                    path: path.to_path_buf(),
                    reason: "the written overlay could not be read back".into(),
                })?;
            self.trust
                .approve(path, &written)
                .map_err(|reason| ConfigPatchError::Trust {
                    path: path.to_path_buf(),
                    reason,
                })?;
        }
        Ok(ConfigPatchReceipt {
            path: path.to_path_buf(),
            created: existing.is_none(),
        })
    }

    /// The patched document must resolve and validate as a configuration
    /// before anything is written. An overlay is validated standalone: every
    /// section defaults, so a partial document is a valid one.
    fn check(&self, path: &Path, document: &Value) -> Result<(), ConfigPatchError> {
        let invalid = |reason| ConfigPatchError::Invalid {
            path: path.to_path_buf(),
            reason,
        };
        let resolved = self
            .validator
            .resolve(document.clone(), path)
            .map_err(invalid)?;
        self.validator.validate(&resolved).map_err(invalid)
    }
}

impl std::fmt::Debug for PatchConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatchConfiguration").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "patch_configuration_tests.rs"]
mod tests;
