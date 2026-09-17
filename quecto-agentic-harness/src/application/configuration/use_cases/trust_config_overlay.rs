//! Approve the repo-local overlay explicitly (#2024): the non-interactive
//! counterpart of the startup prompt, so an agent or a script can trust an
//! overlay it has reviewed. Only an overlay that would actually apply is
//! approved: a JSON object with no global-only section that validates on
//! its own.

use std::sync::Arc;

use serde_json::Value;

use crate::application::configuration::dto::{OverlayTrustError, OverlayTrustRequest};
use crate::application::configuration::overlay_policy::global_only_key;
use crate::application::configuration::ports::{
    ConfigDocumentStore, ConfigValidator, OverlayApproval, OverlayTrustStore,
};

pub struct TrustConfigOverlay {
    store: Arc<dyn ConfigDocumentStore>,
    validator: Arc<dyn ConfigValidator>,
    trust: Arc<dyn OverlayTrustStore>,
}

impl TrustConfigOverlay {
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
        request: OverlayTrustRequest,
    ) -> Result<OverlayApproval, OverlayTrustError> {
        let path = request.path.as_path();
        let bytes = self
            .store
            .read(path)
            .map_err(|reason| OverlayTrustError::Read {
                path: path.to_path_buf(),
                reason,
            })?
            .ok_or_else(|| OverlayTrustError::Missing(path.to_path_buf()))?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|error| OverlayTrustError::Parse {
                path: path.to_path_buf(),
                reason: error.to_string(),
            })?;
        let Some(object) = value.as_object() else {
            return Err(OverlayTrustError::NotAnObject(path.to_path_buf()));
        };
        if let Some(key) = global_only_key(object) {
            return Err(OverlayTrustError::GlobalOnlyKey {
                path: path.to_path_buf(),
                key: key.to_string(),
            });
        }
        let invalid = |reason| OverlayTrustError::Invalid {
            path: path.to_path_buf(),
            reason,
        };
        let resolved = self.validator.resolve(value, path).map_err(invalid)?;
        self.validator.validate_layer(&resolved).map_err(invalid)?;
        self.trust
            .approve(path, &bytes)
            .map_err(|reason| OverlayTrustError::Store {
                path: path.to_path_buf(),
                reason,
            })
    }
}

impl std::fmt::Debug for TrustConfigOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustConfigOverlay").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "trust_config_overlay_tests.rs"]
mod tests;
