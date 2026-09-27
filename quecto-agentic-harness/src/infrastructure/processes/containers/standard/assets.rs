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
pub const STANDARD_ASSET_VERSION: u32 = 6;

/// SHA-256 digests of bundle scripts an earlier quecto wrote (#2206):
/// version 5's `inspect.sh` and `kill.sh`, and its `create.sh` as shipped
/// before #2184 (d950204ee) and before #2173 (cc08db65b). A file holding
/// exactly those bytes is `Outdated`, not an edit: quecto wrote them, so
/// replacing them loses nothing a person wrote. Any other bytes stay
/// `Differs`. Never list the current bytes here.
const PREVIOUSLY_SHIPPED: &[(&str, &str)] = &[
    (
        "scripts/create.sh",
        "008c19f962a358eb1b1c376991c9d08b42a10a0d9e1dd7b70d34cef6e478524b",
    ),
    (
        "scripts/create.sh",
        "34fe6aa6f9e6010de7f513d85c8774c10b16805abe0a619ad11d959bf5961995",
    ),
    (
        "scripts/inspect.sh",
        "395805c76c2a9d9161bec1f293a56582a7b0f3a117ef5f18a1d98fc74d76012a",
    ),
    (
        "scripts/kill.sh",
        "ca6c918ef8b87488737445224bbf8638cfaac855ccc9848d98583036accb736c",
    ),
];

/// Whether `bytes` are what an earlier quecto wrote at the bundle's
/// `path` (a script the bundle owns).
fn previously_shipped(asset: &ContainerAsset, bytes: &[u8]) -> bool {
    use sha2::Digest;
    let digest: String = sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    asset.ownership == AssetOwnership::Bundle
        && PREVIOUSLY_SHIPPED
            .iter()
            .any(|(path, known)| *path == asset.path && *known == digest)
}

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
                } else if previously_shipped(asset, &bytes) {
                    AssetState::Outdated
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

    fn refresh_outdated(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetOutcome, String> {
        let destination = dir.join(&asset.path);
        match self.observe(root, dir, asset)? {
            AssetState::Identical => Ok(AssetOutcome::KeptIdentical),
            AssetState::Outdated => self.place(root, dir, asset, false),
            AssetState::Differs => Err(format!(
                "{} no longer holds the bytes an earlier quecto wrote (edited since it was judged outdated?); left as it is",
                destination.display()
            )),
            AssetState::Missing => Err(format!(
                "{} is gone; run `quecto container init` to materialise it",
                destination.display()
            )),
            AssetState::Refused => Err(format!(
                "{} is not a regular file; refusing to write it",
                destination.display()
            )),
        }
    }
}

/// What an outdated file was when it was re-judged under its own handle
/// (#2206): the replacement renames over it only while it still is.
struct OutdatedSnapshot {
    device: u64,
    inode: u64,
    length: u64,
    modified: Option<std::time::SystemTime>,
}

impl OutdatedSnapshot {
    /// Open `destination` (never through a link), read and hash its bytes
    /// through that one handle, and keep the handle's identity: only a
    /// file that still holds exactly bytes an earlier quecto shipped.
    fn take(destination: &Path, asset: &ContainerAsset) -> Result<Self, String> {
        use std::io::Read;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(destination)
            .map_err(|error| format!("cannot open {}: {error}", destination.display()))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read {}: {error}", destination.display()))?;
        match previously_shipped(asset, &bytes) {
            true => {}
            false => {
                return Err(format!(
                    "{} no longer holds the bytes an earlier quecto wrote (edited since it was judged outdated?); left as it is",
                    destination.display()
                ));
            }
        }
        let meta = file
            .metadata()
            .map_err(|error| format!("cannot stat {}: {error}", destination.display()))?;
        Ok(Self {
            device: meta.dev(),
            inode: meta.ino(),
            length: meta.len(),
            modified: meta.modified().ok(),
        })
    }

    /// Whether `destination` is still the very file this snapshot judged:
    /// same device, inode, length and modification time.
    fn still_holds(&self, destination: &Path) -> Result<(), String> {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::symlink_metadata(destination)
            .map_err(|error| format!("cannot stat {}: {error}", destination.display()))?;
        let same = meta.dev() == self.device
            && meta.ino() == self.inode
            && meta.len() == self.length
            && meta.modified().ok() == self.modified;
        match same {
            true => Ok(()),
            false => Err(format!(
                "{} changed while it was being refreshed; left as it is",
                destination.display()
            )),
        }
    }
}

/// A test's hook, run at each step of placing an asset (`"observe"`
/// before the destination is judged, `"snapshot"` before an outdated file
/// is re-read, `"rename"` before it is renamed over), to change the file
/// in that window.
#[cfg(test)]
pub(crate) type PlaceHook = fn(&Path, &str);

#[cfg(test)]
pub(crate) static PLACE_HOOK: std::sync::Mutex<Option<PlaceHook>> = std::sync::Mutex::new(None);

#[cfg(test)]
fn place_hook(destination: &Path, step: &str) {
    let hook = *PLACE_HOOK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hook) = hook {
        hook(destination, step);
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
        let destination = dir.join(&asset.path);
        #[cfg(test)]
        place_hook(&destination, "observe");
        let (replacing, outdated) = match self.observe(root, dir, asset)? {
            AssetState::Identical => return Ok(AssetOutcome::KeptIdentical),
            AssetState::Differs if !replace => return Ok(AssetOutcome::KeptDiffering),
            AssetState::Differs => (true, None),
            // Bytes an earlier quecto wrote: replaced whatever the run,
            // since no one's edit is lost (#2206) — re-judged through its
            // own handle now, and renamed over only while unchanged.
            AssetState::Outdated => {
                #[cfg(test)]
                place_hook(&destination, "snapshot");
                (true, Some(OutdatedSnapshot::take(&destination, asset)?))
            }
            AssetState::Refused => {
                return Err(format!(
                    "{} is not a regular file; refusing to write it",
                    dir.join(&asset.path).display()
                ));
            }
            AssetState::Missing => (false, None),
        };
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
            // An outdated file is renamed over only while it is still the
            // file judged outdated (#2206): an edit since then stands.
            if let Some(snapshot) = &outdated {
                #[cfg(test)]
                place_hook(&destination, "rename");
                snapshot.still_holds(&destination)?;
            }
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
