//! Inspect effective configuration (#1843): `quecto status` shows where
//! the configuration comes from, what a run would use (workspace, model,
//! effort), which provider keys are configured and which credentials are
//! stored. Read-only: it never prompts for overlay trust and writes
//! nothing.
//!
//! The effective document is [`ResolveEffectiveConfig`]'s answer, an
//! intra-capability call; stored credentials come from the auth
//! capability through the [`StoredCredentialPresence`] port.

use std::sync::Arc;

use crate::application::configuration::dto::{
    ConfigSelection, ConfigurationInspection, InspectionError,
};
use crate::application::configuration::ports::{RunSettingsSource, StoredCredentialPresence};
use crate::application::configuration::use_cases::ResolveEffectiveConfig;

pub struct InspectEffectiveConfiguration {
    resolve: Arc<ResolveEffectiveConfig>,
    settings: Arc<dyn RunSettingsSource>,
    credentials: Arc<dyn StoredCredentialPresence>,
}

impl InspectEffectiveConfiguration {
    pub fn new(
        resolve: Arc<ResolveEffectiveConfig>,
        settings: Arc<dyn RunSettingsSource>,
        credentials: Arc<dyn StoredCredentialPresence>,
    ) -> Self {
        Self {
            resolve,
            settings,
            credentials,
        }
    }

    /// The effective configuration `selection` yields. A layer that cannot
    /// be resolved, or a merged document that does not realize, refuses;
    /// an unreadable credential store is reported in the result.
    pub fn execute(
        &self,
        selection: &ConfigSelection,
    ) -> Result<ConfigurationInspection, InspectionError> {
        // Red: the inspection is not built yet.
        let _ = (&self.resolve, &self.settings, &self.credentials, selection);
        Err(InspectionError::Settings("not inspected yet".to_string()))
    }
}

impl std::fmt::Debug for InspectEffectiveConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectEffectiveConfiguration")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "inspect_effective_configuration_tests.rs"]
mod tests;
