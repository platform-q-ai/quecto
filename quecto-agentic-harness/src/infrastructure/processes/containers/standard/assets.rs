//! The embedded standard bundle behind the environments capability's
//! [`ContainerAssetStore`] port (#2024 S4e). The runtime scripts ARE the
//! official adapter set under `scripts/container-runtime/` (its
//! container-engine subdirectory) — one source, compiled in, so a
//! materialised bundle honours the same preflight (`--preflight-only`,
//! S4b) and swarm (`QUECTO_SWARM_*`, S4c) contracts the repository's own
//! copy does — plus the Containerfile of the image they launch by default
//! and the command that builds it (`build-command`, a template the bundle
//! owns so the runtime's name never enters this crate). Materialisation
//! is create-only: a
//! missing file is written whole (temporary file, fsync, rename, with
//! its mode); an existing file is compared and left alone — unless the
//! run is a refresh, which renames the embedded bytes over a differing
//! file; a symbolic link in the destination's place is refused rather
//! than followed.

use std::io::Write;
use std::path::Path;

use crate::application::environments::dto::{
    AssetOutcome, AssetOwnership, AssetState, ContainerAsset, ContainerAssetCatalogue,
};
use crate::application::environments::ports::ContainerAssetStore;

/// Bumped when an embedded asset changes, so `status` can say which
/// bundle a project carries.
pub const STANDARD_ASSET_VERSION: u32 = 5;

// The scripts are symbolic links to `scripts/container-runtime/` in the
// workspace, resolved by `include_str!` at compile time: one source for
// the repository's copy and the embedded one. `cargo install --path` and
// workspace builds see them; a `cargo package` of this crate alone would
// not carry the link targets, and a checkout with `core.symlinks=false`
// would embed the link text — neither is a supported build of this crate.
const CONTAINERFILE: &str = include_str!("../../../../../assets/standard-container/Containerfile");
const BUILD_COMMAND: &str = include_str!("../../../../../assets/standard-container/build-command");
const CREATE: &str = include_str!("../../../../../assets/standard-container/scripts/create.sh");
const EXEC: &str = include_str!("../../../../../assets/standard-container/scripts/exec.sh");
const INSPECT: &str = include_str!("../../../../../assets/standard-container/scripts/inspect.sh");
const KILL: &str = include_str!("../../../../../assets/standard-container/scripts/kill.sh");

/// `(relative path, contents, executable, ownership)` of every asset, in the
/// order init writes and status lists them. The Containerfile is a starter
/// the project then owns (#2073); the scripts are the trusted adapter.
const ASSETS: &[(&str, &str, bool, AssetOwnership)] = &[
    (
        "Containerfile",
        CONTAINERFILE,
        false,
        AssetOwnership::Project,
    ),
    ("scripts/create.sh", CREATE, true, AssetOwnership::Bundle),
    ("scripts/exec.sh", EXEC, true, AssetOwnership::Bundle),
    ("scripts/inspect.sh", INSPECT, true, AssetOwnership::Bundle),
    ("scripts/kill.sh", KILL, true, AssetOwnership::Bundle),
];

#[derive(Debug, Default, Clone, Copy)]
pub struct EmbeddedStandardAssets;

impl ContainerAssetStore for EmbeddedStandardAssets {
    fn catalogue(&self) -> ContainerAssetCatalogue {
        ContainerAssetCatalogue {
            version: STANDARD_ASSET_VERSION,
            build_command: BUILD_COMMAND.to_string(),
            assets: ASSETS
                .iter()
                .map(|(path, contents, executable, ownership)| ContainerAsset {
                    path: (*path).to_string(),
                    contents: contents.as_bytes().to_vec(),
                    executable: *executable,
                    ownership: *ownership,
                })
                .collect(),
        }
    }

    fn observe(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetState, String> {
        let destination = dir.join(&asset.path);
        if let Some(parent) = destination.parent() {
            refuse_symlinked_directories(root, parent)?;
        }
        match std::fs::symlink_metadata(&destination) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AssetState::Missing),
            Err(error) => Err(format!("cannot stat {}: {error}", destination.display())),
            Ok(meta) if meta.file_type().is_symlink() => Err(format!(
                "{} is a symbolic link; refusing to read through it",
                destination.display()
            )),
            Ok(meta) if !meta.is_file() => Err(format!(
                "{} exists but is not a regular file",
                destination.display()
            )),
            Ok(_) => {
                let bytes = std::fs::read(&destination)
                    .map_err(|error| format!("cannot read {}: {error}", destination.display()))?;
                Ok(if bytes == asset.contents {
                    AssetState::Identical
                } else {
                    AssetState::Differs
                })
            }
        }
    }

    fn materialise(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetOutcome, String> {
        self.place(root, dir, asset, false)
    }

    fn refresh(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetOutcome, String> {
        self.place(root, dir, asset, true)
    }
}

impl EmbeddedStandardAssets {
    /// Write the asset if missing — or, with `replace`, also over a
    /// differing regular file (a rename onto it, so a reader sees the old
    /// bytes or the new, never a torn file).
    fn place(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
        replace: bool,
    ) -> Result<AssetOutcome, String> {
        let replacing = match self.observe(root, dir, asset)? {
            AssetState::Identical => return Ok(AssetOutcome::KeptIdentical),
            AssetState::Differs if !replace => return Ok(AssetOutcome::KeptDiffering),
            AssetState::Differs => true,
            AssetState::Refused => {
                return Err(format!(
                    "{} is not a regular file; refusing to write it",
                    dir.join(&asset.path).display()
                ));
            }
            AssetState::Missing => false,
        };
        let destination = dir.join(&asset.path);
        let parent = destination
            .parent()
            .ok_or_else(|| format!("{} has no parent directory", destination.display()))?;
        refuse_symlinked_directories(root, parent)?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        refuse_symlinked_directories(root, parent)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".quecto-asset-")
            .tempfile_in(parent)
            .map_err(|error| {
                format!(
                    "cannot create a temporary file in {}: {error}",
                    parent.display()
                )
            })?;
        temporary
            .write_all(&asset.contents)
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|error| format!("cannot write {}: {error}", destination.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if asset.executable { 0o755 } else { 0o644 };
            temporary
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(mode))
                .map_err(|error| {
                    format!("cannot set the mode of {}: {error}", destination.display())
                })?;
        }
        if replacing {
            // A refresh renames over the differing file it observed; the
            // link-check just above bounds the race to the file itself.
            return match temporary.persist(&destination) {
                Ok(_) => Ok(AssetOutcome::Refreshed),
                Err(error) => Err(format!(
                    "cannot replace {}: {}",
                    destination.display(),
                    error.error
                )),
            };
        }
        // `persist_noclobber` links the new file in only if nothing took
        // the name meanwhile (a concurrent init): then the other's file
        // stands and ours is dropped.
        match temporary.persist_noclobber(&destination) {
            Ok(_) => Ok(AssetOutcome::Written),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.observe(root, dir, asset).map(|state| match state {
                    AssetState::Differs => AssetOutcome::KeptDiffering,
                    _ => AssetOutcome::KeptIdentical,
                })
            }
            Err(error) => Err(format!(
                "cannot place {}: {}",
                destination.display(),
                error.error
            )),
        }
    }
}

/// No existing directory below `root` (exclusive) down to `parent` may
/// be a symbolic link: `.quecto`, `containers` or the bundle directory
/// swapped for a link would otherwise redirect the materialised files
/// outside the project (the overlay loader refuses the same components
/// on its way to `.quecto/config.json`). Checked before the directories
/// are created and again after, so a link that appeared in between is
/// caught too (the file itself is placed with no-clobber).
fn refuse_symlinked_directories(root: &Path, parent: &Path) -> Result<(), String> {
    let mut current = parent;
    while current != root && current.starts_with(root) {
        if let Ok(meta) = std::fs::symlink_metadata(current)
            && meta.file_type().is_symlink()
        {
            return Err(format!(
                "{} is a symbolic link; refusing to write through it",
                current.display()
            ));
        }
        match current.parent() {
            Some(next) => current = next,
            None => break,
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "assets_tests.rs"]
mod tests;
