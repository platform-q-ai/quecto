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
//! its mode); an existing file is compared and left alone; a symbolic
//! link in the destination's place is refused rather than followed.

use std::io::Write;
use std::path::Path;

use crate::application::environments::dto::{
    AssetOutcome, AssetState, ContainerAsset, ContainerAssetCatalogue,
};
use crate::application::environments::ports::ContainerAssetStore;

/// Bumped when an embedded asset changes, so `status` can say which
/// bundle a project carries.
pub const STANDARD_ASSET_VERSION: u32 = 1;

const CONTAINERFILE: &str = include_str!("../../../../../assets/standard-container/Containerfile");
const BUILD_COMMAND: &str = include_str!("../../../../../assets/standard-container/build-command");
const CREATE: &str = include_str!("../../../../../assets/standard-container/scripts/create.sh");
const EXEC: &str = include_str!("../../../../../assets/standard-container/scripts/exec.sh");
const INSPECT: &str = include_str!("../../../../../assets/standard-container/scripts/inspect.sh");
const KILL: &str = include_str!("../../../../../assets/standard-container/scripts/kill.sh");

/// `(relative path, contents, executable)` of every asset, in the order
/// init writes and status lists them.
const ASSETS: &[(&str, &str, bool)] = &[
    ("Containerfile", CONTAINERFILE, false),
    ("scripts/create.sh", CREATE, true),
    ("scripts/exec.sh", EXEC, true),
    ("scripts/inspect.sh", INSPECT, true),
    ("scripts/kill.sh", KILL, true),
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
                .map(|(path, contents, executable)| ContainerAsset {
                    path: (*path).to_string(),
                    contents: contents.as_bytes().to_vec(),
                    executable: *executable,
                })
                .collect(),
        }
    }

    fn observe(&self, dir: &Path, asset: &ContainerAsset) -> Result<AssetState, String> {
        let destination = dir.join(&asset.path);
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

    fn materialise(&self, dir: &Path, asset: &ContainerAsset) -> Result<AssetOutcome, String> {
        match self.observe(dir, asset)? {
            AssetState::Identical => return Ok(AssetOutcome::KeptIdentical),
            AssetState::Differs => return Ok(AssetOutcome::KeptDiffering),
            AssetState::Missing => {}
        }
        let destination = dir.join(&asset.path);
        let parent = destination
            .parent()
            .ok_or_else(|| format!("{} has no parent directory", destination.display()))?;
        refuse_symlinked_directories(dir, parent)?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        refuse_symlinked_directories(dir, parent)?;
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
        // `persist_noclobber` links the new file in only if nothing took
        // the name meanwhile (a concurrent init): then the other's file
        // stands and ours is dropped.
        match temporary.persist_noclobber(&destination) {
            Ok(_) => Ok(AssetOutcome::Written),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.observe(dir, asset).map(|state| match state {
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

/// No existing directory from `dir` down to `parent` may be a symbolic
/// link: a bundle directory swapped for a link would otherwise redirect
/// the materialised files outside the project. Checked before the
/// directories are created and again after, so a link that appeared in
/// between is caught too (the file itself is placed with no-clobber).
fn refuse_symlinked_directories(dir: &Path, parent: &Path) -> Result<(), String> {
    let mut current = parent;
    loop {
        if let Ok(meta) = std::fs::symlink_metadata(current)
            && meta.file_type().is_symlink()
        {
            return Err(format!(
                "{} is a symbolic link; refusing to write through it",
                current.display()
            ));
        }
        if current == dir {
            return Ok(());
        }
        match current.parent() {
            Some(next) if current.starts_with(dir) => current = next,
            _ => return Ok(()),
        }
    }
}

#[cfg(test)]
#[path = "assets_tests.rs"]
mod tests;
