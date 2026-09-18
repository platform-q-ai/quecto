//! Resolve the effective configuration document of a run (#2024): the
//! base file (explicit or global) with the working directory's overlay
//! merged over it when — and only when — that overlay is trusted (recorded,
//! or consented to at the prompt an interactive adapter offers), carries no
//! global-only section, and is valid as a layer. An untrusted overlay is
//! reported, never applied; a trusted one that is broken is an error naming
//! the file, never a fallback; a merge that is invalid names both files.
//! An overlay the store's overlay policy refuses (a symbolic link on the
//! way to it) is reported refused whatever it holds: trust is keyed by the
//! file's identity, and a link would borrow its target's.
//!
//! [`ResolveEffectiveConfig::preview`] answers the same question for a
//! layer that has not been written yet, so the patch use case validates
//! the merge a write would produce before it writes.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::application::configuration::dto::{
    ConfigLayer, ConfigSelection, ConfigSources, EffectiveConfig, EffectiveConfigError,
    OverlayReport, OverlayState,
};
use crate::application::configuration::overlay_policy::{
    global_only_key, looks_like_config, merge_overlay,
};
use crate::application::configuration::ports::{
    ConfigDocumentStore, ConfigValidator, OverlayDocument, OverlayTrust, OverlayTrustStore,
};

/// The merged document, the overlay's report, and the applied overlay's
/// own document (`None` when it was not applied).
type OverlaidDocument = (
    Map<String, Value>,
    Option<OverlayReport>,
    Option<Map<String, Value>>,
);

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
        self.resolve(selection, None)
    }

    /// The effective configuration `selection` would produce if `document`
    /// (already resolved against its own path) stood in for the `layer`
    /// file — the file itself is not read for that layer. A substituted
    /// overlay is taken as trusted: the caller is about to write and
    /// approve it.
    pub fn preview(
        &self,
        selection: &ConfigSelection,
        layer: ConfigLayer,
        document: &Map<String, Value>,
    ) -> Result<EffectiveConfig, EffectiveConfigError> {
        self.resolve(selection, Some((layer, document)))
    }

    fn resolve(
        &self,
        selection: &ConfigSelection,
        substitute: Option<(ConfigLayer, &Map<String, Value>)>,
    ) -> Result<EffectiveConfig, EffectiveConfigError> {
        let substituted = |layer: ConfigLayer| {
            substitute
                .filter(|(substituted, _)| *substituted == layer)
                .map(|(_, document)| document.clone())
        };
        match selection {
            ConfigSelection::Explicit(path) => {
                let document = match substituted(ConfigLayer::Global) {
                    Some(document) => document,
                    None => {
                        let Some(bytes) = self.read(path)? else {
                            return Err(EffectiveConfigError::Missing(path.clone()));
                        };
                        self.resolved_object(path, &bytes)?
                    }
                };
                self.validate(path, &document)?;
                Ok(EffectiveConfig {
                    document: Value::Object(document),
                    sources: ConfigSources {
                        base: path.clone(),
                        explicit: true,
                        overlay: None,
                        legacy_local: None,
                    },
                    overlay_document: None,
                })
            }
            ConfigSelection::Layered(layers) => {
                // The global file must be valid on its own, whatever the
                // overlay adds: the writer, tool-policy persistence and the
                // admission broker all load it alone.
                let (global, global_present) = match substituted(ConfigLayer::Global) {
                    Some(global) => {
                        self.validate(&layers.global, &global)?;
                        (global, true)
                    }
                    None => match self.read(&layers.global)? {
                        Some(bytes) => {
                            let global = self.resolved_object(&layers.global, &bytes)?;
                            self.validate(&layers.global, &global)?;
                            (global, true)
                        }
                        None => (Map::new(), false),
                    },
                };
                let (document, overlay, overlay_document) =
                    match (&layers.overlay, substituted(ConfigLayer::Overlay)) {
                        (Some(path), Some(overlay)) => {
                            let overlay = self.checked_overlay_object(path, overlay)?;
                            (
                                merge_overlay(global, overlay.clone()),
                                Some(OverlayReport {
                                    path: path.clone(),
                                    state: OverlayState::Applied,
                                }),
                                Some(overlay),
                            )
                        }
                        (Some(path), None) => self.apply_overlay(global, path)?,
                        (None, _) => (global, None, None),
                    };
                match overlay
                    .as_ref()
                    .filter(|report| report.state == OverlayState::Applied)
                {
                    Some(applied) => self
                        .validator
                        .validate(&Value::Object(document.clone()))
                        .map_err(|reason| EffectiveConfigError::InvalidMerge {
                            global: global_present.then(|| layers.global.clone()),
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
                    overlay_document,
                })
            }
        }
    }

    /// The global document with the overlay at `path` merged in when it is
    /// present, trusted and acceptable (the applied overlay travels as the
    /// third element); otherwise the global document and the reason the
    /// overlay did not apply.
    fn apply_overlay(
        &self,
        global: Map<String, Value>,
        path: &Path,
    ) -> Result<OverlaidDocument, EffectiveConfigError> {
        let report = |state| {
            Some(OverlayReport {
                path: path.to_path_buf(),
                state,
            })
        };
        // The store's refusal comes before any trust decision: a link's
        // canonical path is its target's, whose approval it must not inherit.
        let bytes =
            match self
                .store
                .read_overlay(path)
                .map_err(|reason| EffectiveConfigError::Read {
                    path: path.to_path_buf(),
                    reason,
                })? {
                OverlayDocument::Present(bytes) => bytes,
                OverlayDocument::Absent => {
                    return Ok((global, report(OverlayState::Absent), None));
                }
                OverlayDocument::Refused { reason } => {
                    return Ok((global, report(OverlayState::Refused { reason }), None));
                }
            };
        let trust = self.trust.decide(path, &bytes);
        // An untrusted overlay is checked before anyone is asked about it,
        // so nobody is offered — or sent to `quecto config trust` for — a
        // file that would be refused anyway; the refusal travels with the
        // report.
        let overlay = match &trust {
            OverlayTrust::Trusted => self.checked_overlay(path, &bytes)?,
            OverlayTrust::Untrusted { fingerprint } => match self.checked_overlay(path, &bytes) {
                Ok(overlay) if self.trust.offer(path, fingerprint, &bytes) => overlay,
                checked => {
                    // The withheld document's top-level keys travel with
                    // the report: a reader may learn what the overlay
                    // declares without any of it being applied. A document
                    // the checks refused declares nothing.
                    let sections = checked
                        .as_ref()
                        .map(|overlay| overlay.keys().cloned().collect())
                        .unwrap_or_default();
                    return Ok((
                        global,
                        report(OverlayState::Untrusted {
                            fingerprint: fingerprint.clone(),
                            problem: checked.err().map(|error| error.to_string()),
                            sections,
                        }),
                        None,
                    ));
                }
            },
        };
        // Consent given at the prompt is recorded only now, after the same
        // checks `quecto config trust` applies.
        if matches!(trust, OverlayTrust::Untrusted { .. }) {
            self.trust
                .approve(path, &bytes)
                .map_err(|reason| EffectiveConfigError::Read {
                    path: path.to_path_buf(),
                    reason: format!("could not record trust: {reason}"),
                })?;
        }
        Ok((
            merge_overlay(global, overlay.clone()),
            report(OverlayState::Applied),
            Some(overlay),
        ))
    }

    /// The overlay parsed, resolved, free of global-only sections and valid
    /// as a layer.
    fn checked_overlay(
        &self,
        path: &Path,
        bytes: &[u8],
    ) -> Result<Map<String, Value>, EffectiveConfigError> {
        let overlay = self.resolved_object(path, bytes)?;
        self.checked_overlay_object(path, overlay)
    }

    /// An already-resolved overlay, free of global-only sections and valid
    /// as a layer.
    fn checked_overlay_object(
        &self,
        path: &Path,
        overlay: Map<String, Value>,
    ) -> Result<Map<String, Value>, EffectiveConfigError> {
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
        Ok(overlay)
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
