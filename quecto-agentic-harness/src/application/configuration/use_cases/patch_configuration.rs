//! Patch one key of one configuration document (#2024): the safe writer
//! every configuration change goes through.
//!
//! Guarantees: the document is patched as a JSON document (never through
//! the validator's struct, so unknown keys and key order survive), only the
//! addressed path changes, the result must pass validation — the layer on
//! its own, then the effective configuration the run in this directory
//! would load with the patched layer in place — before a byte is written,
//! the whole read → patch → validate → write cycle runs under the
//! writer's exclusive hold on the file so concurrent patches serialise,
//! and the write is all-or-nothing through the store. An overlay patch
//! additionally refuses global-only keys, refuses whatever the store's
//! overlay policy refuses (a symbolic link on the way to the file — checked
//! under the hold, which itself creates nothing on the way), refuses to
//! touch an overlay whose current content is not trusted, and records
//! trust for exactly the bytes the writer laid down.
//!
//! `unset` (#2024 S2) is the rollback of `execute`: the same cycle with
//! the key removed instead of set; a key the layer does not set is an
//! error, so a rollback aimed at the wrong layer says so.
//!
//! The merge check is [`ResolveEffectiveConfig`]'s: a use case of the same
//! capability invoked directly — an intra-capability call, not a
//! cross-capability one (those go through ports).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::application::configuration::dto::{
    ConfigLayer, ConfigPatch, ConfigPatchError, ConfigPatchReceipt, ConfigSelection, ConfigUnset,
};
use crate::application::configuration::overlay_policy::{
    GLOBAL_ONLY_KEYS, key_segments, remove_path, set_path,
};
use crate::application::configuration::ports::{
    ConfigDocumentStore, ConfigDocumentWriter, ConfigValidator, OverlayDocument, OverlayTrust,
    OverlayTrustStore,
};
use crate::application::configuration::use_cases::ResolveEffectiveConfig;

pub struct PatchConfiguration {
    store: Arc<dyn ConfigDocumentStore>,
    writer: Arc<dyn ConfigDocumentWriter>,
    validator: Arc<dyn ConfigValidator>,
    trust: Arc<dyn OverlayTrustStore>,
    resolve: Arc<ResolveEffectiveConfig>,
}

impl PatchConfiguration {
    pub fn new(
        store: Arc<dyn ConfigDocumentStore>,
        writer: Arc<dyn ConfigDocumentWriter>,
        validator: Arc<dyn ConfigValidator>,
        trust: Arc<dyn OverlayTrustStore>,
        resolve: Arc<ResolveEffectiveConfig>,
    ) -> Self {
        Self {
            store,
            writer,
            validator,
            trust,
            resolve,
        }
    }

    pub fn execute(&self, patch: ConfigPatch) -> Result<ConfigPatchReceipt, ConfigPatchError> {
        let ConfigPatch {
            selection,
            layer,
            key_path,
            value,
        } = patch;
        self.cycle(&selection, layer, &key_path, |document, path| {
            set_path(document, &key_path, value).map_err(|at| ConfigPatchError::NotAnObject {
                path: path.to_path_buf(),
                at,
            })
        })
    }

    /// Remove one key of one layer; the rollback of [`Self::execute`].
    pub fn unset(&self, unset: ConfigUnset) -> Result<ConfigPatchReceipt, ConfigPatchError> {
        let ConfigUnset {
            selection,
            layer,
            key_path,
        } = unset;
        self.cycle(
            &selection,
            layer,
            &key_path,
            |document, path| match remove_path(document, &key_path) {
                Ok(Some(_)) => Ok(()),
                Ok(None) => Err(ConfigPatchError::NotSet {
                    path: path.to_path_buf(),
                    key_path: key_path.clone(),
                }),
                Err(at) => Err(ConfigPatchError::NotAnObject {
                    path: path.to_path_buf(),
                    at,
                }),
            },
        )
    }

    /// The read → mutate → validate → write cycle under the writer's hold.
    fn cycle(
        &self,
        selection: &ConfigSelection,
        layer: ConfigLayer,
        key_path: &str,
        mutate: impl FnOnce(&mut Value, &Path) -> Result<(), ConfigPatchError>,
    ) -> Result<ConfigPatchReceipt, ConfigPatchError> {
        let path = self.target_path(selection, layer)?;
        let path = path.as_path();
        let segments = key_segments(key_path)
            .ok_or_else(|| ConfigPatchError::InvalidKeyPath(key_path.to_string()))?;
        if layer == ConfigLayer::Overlay && GLOBAL_ONLY_KEYS.contains(&segments[0]) {
            return Err(ConfigPatchError::GlobalOnlyKey {
                path: path.to_path_buf(),
                key: segments[0].to_string(),
            });
        }
        // Held until the write has landed (or the patch is refused): what
        // is read below is what the write replaces.
        let _hold = self
            .writer
            .exclusive(path)
            .map_err(|reason| ConfigPatchError::Write {
                path: path.to_path_buf(),
                reason,
            })?;
        let existing = self.existing(layer, path)?;
        if let (ConfigLayer::Overlay, Some(bytes)) = (layer, &existing)
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
            None => Value::Object(Map::new()),
        };
        mutate(&mut document, path)?;
        self.check(selection, layer, path, &document)?;
        let written =
            self.writer
                .write(path, &document)
                .map_err(|reason| ConfigPatchError::Write {
                    path: path.to_path_buf(),
                    reason,
                })?;
        if layer == ConfigLayer::Overlay {
            // Trust is by exact bytes: the ones the writer laid down, never
            // a read-back (another writer may have replaced the file since).
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

    /// The document as it stands: the base file read plainly, the overlay
    /// under the store's overlay policy — a refusal (a symbolic link on the
    /// way to it) is the patch's, and nothing is written through the link.
    fn existing(
        &self,
        layer: ConfigLayer,
        path: &Path,
    ) -> Result<Option<Vec<u8>>, ConfigPatchError> {
        let read_error = |reason| ConfigPatchError::Read {
            path: path.to_path_buf(),
            reason,
        };
        match layer {
            ConfigLayer::Global => self.store.read(path).map_err(read_error),
            ConfigLayer::Overlay => match self.store.read_overlay(path).map_err(read_error)? {
                OverlayDocument::Present(bytes) => Ok(Some(bytes)),
                OverlayDocument::Absent => Ok(None),
                OverlayDocument::Refused { reason } => Err(ConfigPatchError::Refused {
                    path: path.to_path_buf(),
                    reason,
                }),
            },
        }
    }

    /// The file the patch addresses: the selection's base file for the
    /// global layer, its overlay location for the overlay layer.
    fn target_path(
        &self,
        selection: &ConfigSelection,
        layer: ConfigLayer,
    ) -> Result<PathBuf, ConfigPatchError> {
        match layer {
            ConfigLayer::Global => Ok(selection.path().to_path_buf()),
            ConfigLayer::Overlay => selection
                .overlay_path()
                .map(Path::to_path_buf)
                .ok_or(ConfigPatchError::NoOverlayLocation),
        }
    }

    /// The patched document must resolve and validate before anything is
    /// written: the layer on its own (the global file as a complete
    /// configuration, the overlay as one layer), then the effective
    /// configuration it produces with the other layer as it stands on
    /// disk — a layer that is valid alone can still brick the merge (an
    /// overlay container config that leaves no default).
    fn check(
        &self,
        selection: &ConfigSelection,
        layer: ConfigLayer,
        path: &Path,
        document: &Value,
    ) -> Result<(), ConfigPatchError> {
        let invalid = |reason| ConfigPatchError::Invalid {
            path: path.to_path_buf(),
            reason,
        };
        let resolved = self
            .validator
            .resolve(document.clone(), path)
            .map_err(invalid)?;
        match layer {
            ConfigLayer::Global => self.validator.validate(&resolved),
            ConfigLayer::Overlay => self.validator.validate_layer(&resolved),
        }
        .map_err(invalid)?;
        let Value::Object(resolved) = resolved else {
            return Err(ConfigPatchError::NotAnObject {
                path: path.to_path_buf(),
                at: String::new(),
            });
        };
        self.resolve
            .preview(selection, layer, &resolved)
            .map(|_| ())
            .map_err(|reason| ConfigPatchError::InvalidMerge {
                path: path.to_path_buf(),
                reason,
            })
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
