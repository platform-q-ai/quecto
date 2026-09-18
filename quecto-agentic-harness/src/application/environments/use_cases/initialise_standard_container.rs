//! `quecto container init` (#2024 S4e): materialise the standard bundle
//! (Containerfile, the official runtime scripts) below the project and
//! bind the project to it through its repo-local overlay. The use case
//! decides where the bundle goes (`<project>/.quecto/containers/standard`),
//! what the entry says (the script paths exactly where the assets were
//! materialised, the state dir, the repository baked in, the image), and
//! whether the entry is the default (only when the effective set has no
//! other default: a valid configuration labels exactly one). The
//! repository is `--repo`, else the checkout's `origin` remote, else the
//! entry is a sandbox and the report says so. Nothing here reads a file
//! or runs a program: the asset store, the origin and the persistence
//! ports do, and composition maps the persistence onto the configuration
//! capability's safe writer, so trust is recorded for exactly the bytes
//! written and an untrusted overlay is refused, never adopted.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::application::environments::dto::{
    AssetOutcome, AssetState, ContainerAsset, ContainerConfigDocument,
    InitialiseStandardContainerError, RepositoryOrigin, STANDARD_CONTAINER_CONFIG,
    STANDARD_CONTAINER_DIR, STANDARD_CONTAINER_IMAGE, StandardContainerReport,
    StandardContainerRequest, StandardEntryOutcome,
};
use crate::application::environments::ports::{
    ContainerAssetStore, ContainerConfigPersistence, ContainerConfigRoster, WorkspaceOrigin,
};

/// The relative script paths the entry's argv names, as the catalogue
/// carries them.
const CREATE_SCRIPT: &str = "scripts/create.sh";
const EXEC_SCRIPT: &str = "scripts/exec.sh";
const INSPECT_SCRIPT: &str = "scripts/inspect.sh";
const KILL_SCRIPT: &str = "scripts/kill.sh";

/// Where environments' state lives, below the quecto base directory.
pub const STATE_DIR_NAME: &str = "container-environments";

pub struct InitialiseStandardContainer {
    assets: Arc<dyn ContainerAssetStore>,
    origin: Arc<dyn WorkspaceOrigin>,
    roster: Arc<dyn ContainerConfigRoster>,
    persistence: Arc<dyn ContainerConfigPersistence>,
    /// The quecto base directory the state dir is placed under.
    base_dir: PathBuf,
}

impl InitialiseStandardContainer {
    pub fn new(
        assets: Arc<dyn ContainerAssetStore>,
        origin: Arc<dyn WorkspaceOrigin>,
        roster: Arc<dyn ContainerConfigRoster>,
        persistence: Arc<dyn ContainerConfigPersistence>,
        base_dir: PathBuf,
    ) -> Self {
        Self {
            assets,
            origin,
            roster,
            persistence,
            base_dir,
        }
    }

    pub fn execute(
        &self,
        request: &StandardContainerRequest,
    ) -> Result<StandardContainerReport, InitialiseStandardContainerError> {
        if !request.project.is_absolute() {
            return Err(InitialiseStandardContainerError::ProjectNotAbsolute(
                request.project.clone(),
            ));
        }
        if !self.base_dir.is_absolute() {
            return Err(InitialiseStandardContainerError::BaseDirNotAbsolute(
                self.base_dir.clone(),
            ));
        }
        let assets_dir = request.project.join(STANDARD_CONTAINER_DIR);
        let catalogue = self.assets.catalogue();
        // The repository and the effective set are resolved before any
        // asset is written: a refusal (an untrusted overlay) leaves the
        // project untouched.
        let (repository, repository_origin) = match &request.repository {
            Some(url) => (Some(url.clone()), RepositoryOrigin::Explicit),
            None => match self
                .origin
                .origin(&request.project)
                .map_err(InitialiseStandardContainerError::Origin)?
            {
                Some(url) if url_carries_userinfo(&url) => {
                    return Err(InitialiseStandardContainerError::OriginCarriesCredentials(
                        crate::domain::redaction::redact_url_userinfo(&url),
                    ));
                }
                Some(url) => (Some(url), RepositoryOrigin::CheckoutOrigin),
                None => (None, RepositoryOrigin::Sandbox),
            },
        };
        let roster = self
            .roster
            .roster()
            .map_err(InitialiseStandardContainerError::Configuration)?;
        if roster.overlay_withheld {
            return Err(InitialiseStandardContainerError::Configuration(format!(
                "container init refused: the checkout's repo-local config overlay is not trusted, so init cannot add to it without adopting its content ({}); review it and run `quecto config trust` first",
                roster.diagnostics.join("; ")
            )));
        }
        // The write's own refusal (no overlay location, an untrusted
        // overlay of any content) is asked for before an asset is
        // written, so a refused init leaves the project untouched — on a
        // dry run too, which must promise no success a real run would
        // not deliver.
        self.persistence
            .check()
            .map_err(InitialiseStandardContainerError::Persist)?;
        let existing_default = roster
            .configs
            .iter()
            .find(|entry| entry.default && entry.name != STANDARD_CONTAINER_CONFIG)
            .map(|entry| entry.name.clone());
        let image = request
            .image
            .clone()
            .unwrap_or_else(|| STANDARD_CONTAINER_IMAGE.to_string());
        let entry = self.entry(
            &assets_dir,
            repository.as_deref(),
            &image,
            existing_default.is_none(),
        );

        // The entry first: it is the refusal-prone write, and an asset
        // failure after it is healed by running init again.
        let path = if request.dry_run {
            self.persistence.location()
        } else {
            Some(
                self.persistence
                    .persist(STANDARD_CONTAINER_CONFIG, &entry)
                    .map_err(InitialiseStandardContainerError::Persist)?
                    .path,
            )
        };
        let mut written = Vec::new();
        let mut kept = Vec::new();
        let mut differing = Vec::new();
        for asset in &catalogue.assets {
            let path = assets_dir.join(&asset.path);
            let outcome = if request.dry_run {
                match self.observe(&assets_dir, asset)? {
                    AssetState::Missing => AssetOutcome::Written,
                    AssetState::Identical => AssetOutcome::KeptIdentical,
                    AssetState::Differs => AssetOutcome::KeptDiffering,
                    AssetState::Refused => {
                        return Err(InitialiseStandardContainerError::Asset {
                            path,
                            reason: "the destination is not a regular file".into(),
                        });
                    }
                }
            } else {
                self.assets
                    .materialise(&request.project, &assets_dir, asset)
                    .map_err(|reason| InitialiseStandardContainerError::Asset {
                        path: path.clone(),
                        reason,
                    })?
            };
            match outcome {
                AssetOutcome::Written => written.push(path),
                AssetOutcome::KeptIdentical => kept.push(path),
                AssetOutcome::KeptDiffering => differing.push(path),
            }
        }

        let build_command = catalogue.build_command_for(&image, &assets_dir);
        Ok(StandardContainerReport {
            assets_dir,
            version: catalogue.version,
            written,
            kept,
            differing,
            repository,
            repository_origin,
            build_command,
            image,
            entry: StandardEntryOutcome {
                name: STANDARD_CONTAINER_CONFIG.to_string(),
                path,
                default: existing_default.is_none(),
                existing_default,
                entry,
            },
            dry_run: request.dry_run,
        })
    }

    fn observe(
        &self,
        assets_dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetState, InitialiseStandardContainerError> {
        self.assets.observe(assets_dir, asset).map_err(|reason| {
            InitialiseStandardContainerError::Asset {
                path: assets_dir.join(&asset.path),
                reason,
            }
        })
    }

    /// The entry: every argv names the materialised script by absolute
    /// path, `--state-dir` under the base directory, the repository baked
    /// into `create` (#1410), the image explicit, no trailing `--` (the
    /// launcher appends it before the child command).
    fn entry(
        &self,
        assets_dir: &Path,
        repository: Option<&str>,
        image: &str,
        default: bool,
    ) -> ContainerConfigDocument {
        let script = |name: &str| assets_dir.join(name).to_string_lossy().into_owned();
        let state_dir = self
            .base_dir
            .join(STATE_DIR_NAME)
            .to_string_lossy()
            .into_owned();
        let with_state =
            |name: &str| vec![script(name), "--state-dir".to_string(), state_dir.clone()];
        let mut create = with_state(CREATE_SCRIPT);
        if let Some(repository) = repository {
            create.push("--repo".into());
            create.push(repository.to_string());
        }
        create.push("--image".into());
        create.push(image.to_string());
        let mut kill = with_state(KILL_SCRIPT);
        kill.extend(["--op".to_string(), "kill".to_string()]);
        let mut cleanup = with_state(KILL_SCRIPT);
        cleanup.extend(["--op".to_string(), "cleanup".to_string()]);
        ContainerConfigDocument {
            default,
            create,
            exec: with_state(EXEC_SCRIPT),
            inspect: with_state(INSPECT_SCRIPT),
            kill,
            cleanup,
        }
    }
}

/// `scheme://user[:password]@host/…`: credentials in the URL's userinfo.
fn url_carries_userinfo(url: &str) -> bool {
    url.split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or("").contains('@'))
        .unwrap_or(false)
}

impl std::fmt::Debug for InitialiseStandardContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InitialiseStandardContainer")
            .field("base_dir", &self.base_dir)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "initialise_standard_container_tests.rs"]
mod tests;
