//! `quecto container init` (#2024 S4e): materialise the standard bundle
//! (Containerfile, the official runtime scripts) below the project and
//! bind the project to it through its repo-local overlay. The use case
//! decides where the bundle goes (`<project>/.quecto/containers/standard`),
//! what the entry says (the script paths exactly where the assets were
//! materialised, the state dir, the repository baked in, the image), and
//! that the entry is the default — always (#2035: a repo's standard
//! container is its default): a global default is overridden for this
//! repo by the overlay's label and named in the report, and an overlay
//! entry carrying the label loses it in the same write and is named as
//! displaced. The
//! repository is `--repo`, else the checkout's `origin` remote, else the
//! entry is a sandbox and the report says so. Nothing here reads a file
//! or runs a program: the asset store, the origin and the persistence
//! ports do, and composition maps the persistence onto the configuration
//! capability's safe writer, so trust is recorded for exactly the bytes
//! written and an untrusted overlay is refused, never adopted.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::application::environments::dto::{
    AssetOutcome, AssetOwnership, AssetState, ContainerAsset, ContainerConfigDocument,
    ContainerConfigLayer, EntryValueChange, InitialiseStandardContainerError, RepositoryOrigin,
    STANDARD_CONTAINER_CONFIG, STANDARD_CONTAINER_DIR, STANDARD_CONTAINER_IMAGE,
    StandardContainerReport, StandardContainerRequest, StandardEntryOutcome,
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
        // The project must be the checkout's root: an overlay below it
        // binds a subdirectory an agent started at the root never reads.
        if let Some(toplevel) = self
            .origin
            .toplevel(&request.project)
            .map_err(InitialiseStandardContainerError::Origin)?
            && toplevel != request.project
        {
            return Err(InitialiseStandardContainerError::NotRepositoryRoot {
                project: request.project.clone(),
                toplevel,
            });
        }
        let assets_dir = request.project.join(STANDARD_CONTAINER_DIR);
        let catalogue = self.assets.catalogue();
        // The repository and the effective set are resolved before any
        // asset is written: a refusal (an untrusted overlay) leaves the
        // project untouched. An existing standard entry keeps its --repo
        // and --image unless the flag is given: a re-init never resets
        // what an operator chose, and never re-derives the origin over it.
        let existing = self
            .persistence
            .existing(STANDARD_CONTAINER_CONFIG)
            .map_err(InitialiseStandardContainerError::Persist)?;
        let previous_repository = existing
            .as_ref()
            .and_then(|entry| entry.create_value("--repo").map(str::to_string));
        let previous_image = existing
            .as_ref()
            .and_then(|entry| entry.create_value("--image").map(str::to_string));
        let (repository, repository_origin) = match (&request.repository, &existing) {
            (Some(url), _) => (Some(url.clone()), RepositoryOrigin::Explicit),
            (None, Some(_)) => (previous_repository.clone(), RepositoryOrigin::ExistingEntry),
            (None, None) => match self
                .origin
                .origin(&request.project)
                .map_err(InitialiseStandardContainerError::Origin)?
            {
                Some(url) => (Some(url), RepositoryOrigin::CheckoutOrigin),
                None => (None, RepositoryOrigin::Sandbox),
            },
        };
        let image = match (&request.image, &previous_image) {
            (Some(image), _) => image.clone(),
            (None, Some(image)) => image.clone(),
            (None, None) => STANDARD_CONTAINER_IMAGE.to_string(),
        };
        let change = |previous: Option<&String>, current: Option<&String>| {
            existing.as_ref().map(|_| {
                if previous == current {
                    EntryValueChange::Kept
                } else {
                    EntryValueChange::Rewrote {
                        previous: previous.cloned(),
                    }
                }
            })
        };
        let repository_change = change(previous_repository.as_ref(), repository.as_ref());
        let image_change = change(previous_image.as_ref(), Some(&image));
        if let Some(url) = repository
            .as_deref()
            .filter(|url| url_carries_credential(url))
        {
            return Err(
                InitialiseStandardContainerError::RepositoryCarriesCredentials {
                    url: redacted_repository(url),
                    origin: repository_origin,
                },
            );
        }
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
        let labelled_other = |layer: ContainerConfigLayer| {
            roster
                .configs
                .iter()
                .find(|entry| {
                    entry.default && entry.layer == layer && entry.name != STANDARD_CONTAINER_CONFIG
                })
                .map(|entry| entry.name.clone())
        };
        let displaced_default = labelled_other(ContainerConfigLayer::Overlay);
        let overridden_global_default = labelled_other(ContainerConfigLayer::Global);
        let entry = self.entry(&assets_dir, repository.as_deref(), &image);

        // Every destination is judged before anything is written: a
        // symbolic link on the way, a directory in a file's place, is a
        // refusal that leaves the project untouched (a dry run and a
        // status see the same).
        let observed: Vec<AssetState> = catalogue
            .assets
            .iter()
            .map(|asset| self.observe(&request.project, &assets_dir, asset, false))
            .collect::<Result<_, _>>()?;
        // The entry next: it is the other refusal-prone write; an asset
        // failure after it (a race with the filesystem) names the way out.
        let path = if request.dry_run {
            self.persistence.location()
        } else {
            Some(
                self.persistence
                    .persist(
                        STANDARD_CONTAINER_CONFIG,
                        &entry,
                        displaced_default.as_deref(),
                    )
                    .map_err(InitialiseStandardContainerError::Persist)?
                    .path,
            )
        };
        let mut written = Vec::new();
        let mut kept = Vec::new();
        let mut differing = Vec::new();
        let mut refreshed = Vec::new();
        let mut own = Vec::new();
        for (asset, state) in catalogue.assets.iter().zip(observed) {
            let path = assets_dir.join(&asset.path);
            let outcome = if request.dry_run {
                match state {
                    AssetState::Missing => AssetOutcome::Written,
                    AssetState::Identical => AssetOutcome::KeptIdentical,
                    AssetState::Differs if asset.is_projects_own(state) => AssetOutcome::KeptOwn,
                    AssetState::Differs if request.refresh => AssetOutcome::Refreshed,
                    AssetState::Differs => AssetOutcome::KeptDiffering,
                    AssetState::Refused => {
                        return Err(InitialiseStandardContainerError::Asset {
                            path,
                            reason: "the destination is not a regular file".into(),
                            entry_written: false,
                        });
                    }
                }
            } else {
                let store = &self.assets;
                // Only the bundle's own files are ever replaced: a
                // project-owned one is written when missing and otherwise
                // left alone, whatever the run.
                let replace = request.refresh && asset.ownership == AssetOwnership::Bundle;
                if replace {
                    store.refresh(&request.project, &assets_dir, asset)
                } else {
                    store.materialise(&request.project, &assets_dir, asset)
                }
                .map(|outcome| match (asset.ownership, outcome) {
                    (AssetOwnership::Project, AssetOutcome::KeptDiffering) => AssetOutcome::KeptOwn,
                    (AssetOwnership::Project | AssetOwnership::Bundle, outcome) => outcome,
                })
                .inspect(|outcome| {
                    assert!(
                        asset.ownership == AssetOwnership::Bundle
                            || *outcome != AssetOutcome::Refreshed,
                        "a project-owned asset was replaced: {}",
                        asset.path
                    );
                })
                .map_err(|reason| InitialiseStandardContainerError::Asset {
                    path: path.clone(),
                    reason,
                    entry_written: true,
                })?
            };
            match outcome {
                AssetOutcome::Written => written.push(path),
                AssetOutcome::KeptIdentical => kept.push(path),
                AssetOutcome::KeptDiffering => differing.push(path),
                AssetOutcome::Refreshed => refreshed.push(path),
                AssetOutcome::KeptOwn => own.push(path),
            }
        }

        let build_command = catalogue.build_command_for(&image, &assets_dir);
        Ok(StandardContainerReport {
            assets_dir,
            version: catalogue.version,
            written,
            kept,
            differing,
            refreshed,
            own,
            repository,
            repository_origin,
            repository_change,
            image_change,
            build_command,
            image,
            entry: StandardEntryOutcome {
                name: STANDARD_CONTAINER_CONFIG.to_string(),
                path,
                displaced_default,
                overridden_global_default,
                entry,
            },
            dry_run: request.dry_run,
        })
    }

    fn observe(
        &self,
        root: &Path,
        assets_dir: &Path,
        asset: &ContainerAsset,
        entry_written: bool,
    ) -> Result<AssetState, InitialiseStandardContainerError> {
        self.assets
            .observe(root, assets_dir, asset)
            .map_err(|reason| InitialiseStandardContainerError::Asset {
                path: assets_dir.join(&asset.path),
                reason,
                entry_written,
            })
    }

    /// The entry: the default label (#2035), every argv naming the
    /// materialised script by absolute path, `--state-dir` under the base
    /// directory, the repository baked into `create` (#1410), the image
    /// explicit, no trailing `--` (the launcher appends it before the
    /// child command).
    fn entry(
        &self,
        assets_dir: &Path,
        repository: Option<&str>,
        image: &str,
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
            default: true,
            create,
            exec: with_state(EXEC_SCRIPT),
            inspect: with_state(INSPECT_SCRIPT),
            kill,
            cleanup,
        }
    }
}

/// Whether the URL's userinfo is a credential. Over `http`/`https` ANY
/// non-empty userinfo is one: a bare user there is a token (GitHub's
/// documented `https://ghp_xxx@github.com/org/repo` has no colon), so
/// `user@`, `user:password@` and `token@` are all refused. Over the
/// other schemes (`ssh://git@host/…`, `git://…`) and the scp form
/// (`git@host:org/repo`) a bare user names an account, the everyday
/// form, and only a `user:password@` is a secret.
fn url_carries_credential(url: &str) -> bool {
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (Some(scheme.to_ascii_lowercase()), rest),
        None => (None, url),
    };
    let Some((userinfo, _)) = rest.split('/').next().and_then(|authority| {
        // The scp form's `host:path` has no slash before the path.
        authority.rsplit_once('@')
    }) else {
        return false;
    };
    match scheme.as_deref() {
        Some("http" | "https") => !userinfo.is_empty(),
        _ => userinfo.contains(':'),
    }
}

/// The URL with its userinfo replaced by `***`, in the scp form
/// (`user:pw@host:path`, no `://`) as well as the scheme forms.
fn redacted_repository(url: &str) -> String {
    if url.contains("://") {
        return crate::domain::redaction::redact_url_userinfo(url);
    }
    match url.split_once('@') {
        Some((_, rest)) => format!("***@{rest}"),
        None => url.to_string(),
    }
}

impl std::fmt::Debug for InitialiseStandardContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InitialiseStandardContainer")
            .field("base_dir", &self.base_dir)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "initialise_standard_container_ownership_tests.rs"]
mod ownership_tests;
#[cfg(test)]
#[path = "initialise_standard_container_review_tests.rs"]
mod review_tests;
#[cfg(test)]
#[path = "initialise_standard_container_rig_tests.rs"]
mod rig_tests;
#[cfg(test)]
#[path = "initialise_standard_container_tests.rs"]
mod tests;
