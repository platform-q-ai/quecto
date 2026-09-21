//! `quecto container status` (#2024 S4e): where the standard bundle
//! stands for a project — which assets are present and whether they are
//! the embedded ones, whether the effective set carries the `standard`
//! entry, which layer declared it and what it is to `container: true`
//! here (the checkout's own is its default by rule, #2035 — a label
//! removed by hand is diagnosed with the remedy), whether the checkout's overlay was
//! applied, and whether the image is present, asked of the entry's own
//! create preflight (the S4b contract) so status and doctor never
//! disagree. A status is a report, never an error: what could not be
//! established is said in the report's own words.

use std::path::Path;
use std::sync::Arc;

use crate::application::environments::dto::{
    AssetState, CheckStatus, ContainerConfigLayer, ContainerRuntimeTarget,
    STANDARD_CONTAINER_CONFIG, STANDARD_CONTAINER_DIR, StandardContainerStatus, StandardDefault,
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
        let mut asset_diagnostics = Vec::new();
        let mut projects_own = Vec::new();
        let assets = catalogue
            .assets
            .iter()
            .map(|asset| {
                let path = assets_dir.join(&asset.path);
                let state = match self.assets.observe(project, &assets_dir, asset) {
                    Ok(state) => state,
                    // A destination that cannot be judged (a symbolic
                    // link, a directory in a file's place) is not
                    // "missing": init would refuse it, so say why.
                    Err(reason) => {
                        asset_diagnostics.push(reason);
                        AssetState::Refused
                    }
                };
                if asset.is_projects_own(state) {
                    projects_own.push(path.clone());
                }
                (path, state)
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
        let standard_default = entry.as_ref().map(|entry| match entry.layer {
            // The same rule as the listing query: a launch refuses a broken
            // entry before any label or rule is consulted.
            ContainerConfigLayer::Overlay if entry.problem.is_some() => StandardDefault::Refused,
            ContainerConfigLayer::Overlay if entry.default => StandardDefault::RepoDefault,
            ContainerConfigLayer::Overlay => StandardDefault::LabelRemoved,
            ContainerConfigLayer::Global => StandardDefault::GlobalEntry {
                labelled: entry.default,
            },
        });
        if standard_default == Some(StandardDefault::LabelRemoved) {
            diagnostics.push(format!(
                "the overlay's {STANDARD_CONTAINER_CONFIG} entry lost its \"default\": true label (removed with `quecto config unset --local`; a raw edit would have un-trusted the overlay); container: true still selects it (a repo's standard container is its default) — restore the label with `quecto container init --refresh` or `quecto config set --local container_configs.{STANDARD_CONTAINER_CONFIG}.default true`"
            ));
        }
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
        diagnostics.extend(asset_diagnostics);
        StandardContainerStatus {
            assets_dir,
            version: catalogue.version,
            assets,
            projects_own,
            entry,
            standard_default,
            overlay_withheld,
            diagnostics,
            image,
            preflight_error,
        }
    }

    /// The entry's create preflight, reduced to what it says of the image:
    /// the first failed image check (the lookup, a shell and git, the tools
    /// the image declares), or else the lookup itself. A script that reports
    /// only `image` is judged on that alone.
    fn image_check(
        &self,
    ) -> Result<Option<crate::application::environments::dto::PreflightCheck>, String> {
        let config = self.lookup.lookup(&ContainerRuntimeTarget {
            name: Some(STANDARD_CONTAINER_CONFIG.to_string()),
        })?;
        let mut image_checks: Vec<_> = self
            .preflight
            .preflight(&config)?
            .into_iter()
            .filter(|check| IMAGE_CHECKS.contains(&check.name.as_str()))
            .collect();
        let reported = image_checks
            .iter()
            .position(|check| check.status == CheckStatus::Failed)
            .or_else(|| image_checks.iter().position(|check| check.name == "image"));
        Ok(reported.map(|index| image_checks.swap_remove(index)))
    }
}

/// The create preflight's checks that judge the image itself.
const IMAGE_CHECKS: &[&str] = &["image", "image-base", "required-tools"];

impl std::fmt::Debug for ContainerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerStatus").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "container_status_tests.rs"]
mod tests;
