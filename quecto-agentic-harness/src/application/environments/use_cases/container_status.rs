//! `quecto container status` (#2024 S4e): where the standard bundle
//! stands for a project — which assets are present and whether they are
//! the embedded ones, whether the effective set carries the `standard`
//! entry and which layer declared it, whether the checkout's overlay was
//! applied, and whether the image is present, asked of the entry's own
//! create preflight (the S4b contract) so status and doctor never
//! disagree. A status is a report, never an error: what could not be
//! established is said in the report's own words.

use std::path::Path;
use std::sync::Arc;

use crate::application::environments::dto::{
    AssetState, ContainerRuntimeTarget, STANDARD_CONTAINER_CONFIG, STANDARD_CONTAINER_DIR,
    StandardContainerStatus,
};
use crate::application::environments::ports::{
    ContainerAssetStore, ContainerConfigLookup, ContainerConfigRoster, ContainerRuntimePreflight,
};

pub struct ContainerStatus {
    assets: Arc<dyn ContainerAssetStore>,
    roster: Arc<dyn ContainerConfigRoster>,
    lookup: Arc<dyn ContainerConfigLookup>,
    preflight: Arc<dyn ContainerRuntimePreflight>,
}

impl ContainerStatus {
    pub fn new(
        assets: Arc<dyn ContainerAssetStore>,
        roster: Arc<dyn ContainerConfigRoster>,
        lookup: Arc<dyn ContainerConfigLookup>,
        preflight: Arc<dyn ContainerRuntimePreflight>,
    ) -> Self {
        Self {
            assets,
            roster,
            lookup,
            preflight,
        }
    }

    pub fn execute(&self, project: &Path) -> StandardContainerStatus {
        let assets_dir = project.join(STANDARD_CONTAINER_DIR);
        let catalogue = self.assets.catalogue();
        let assets = catalogue
            .assets
            .iter()
            .map(|asset| {
                let state = self
                    .assets
                    .observe(&assets_dir, asset)
                    .unwrap_or(AssetState::Missing);
                (assets_dir.join(&asset.path), state)
            })
            .collect();
        let (entry, overlay_withheld, mut diagnostics) = match self.roster.roster() {
            Ok(report) => (
                report
                    .configs
                    .into_iter()
                    .find(|entry| entry.name == STANDARD_CONTAINER_CONFIG),
                report.overlay_withheld,
                report.diagnostics,
            ),
            Err(reason) => (None, false, vec![reason]),
        };
        let (image, preflight_error) = if entry.is_some() {
            match self.image_check() {
                Ok(check) => (check, None),
                Err(reason) => (None, Some(reason)),
            }
        } else {
            (None, None)
        };
        if let Some(reason) = &preflight_error {
            diagnostics.push(reason.clone());
        }
        StandardContainerStatus {
            assets_dir,
            version: catalogue.version,
            assets,
            entry,
            overlay_withheld,
            diagnostics,
            image,
            preflight_error,
        }
    }

    /// The entry's create preflight, reduced to its `image` check.
    fn image_check(
        &self,
    ) -> Result<Option<crate::application::environments::dto::PreflightCheck>, String> {
        let config = self.lookup.lookup(&ContainerRuntimeTarget {
            name: Some(STANDARD_CONTAINER_CONFIG.to_string()),
        })?;
        let checks = self.preflight.preflight(&config)?;
        Ok(checks.into_iter().find(|check| check.name == "image"))
    }
}

impl std::fmt::Debug for ContainerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerStatus").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "container_status_tests.rs"]
mod tests;
