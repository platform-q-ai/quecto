//! Embedded, versioned standard project assets and safe initialization.
//!
//! The catalog is compiled into the executable.  It deliberately has no
//! runtime dependency on the source checkout: `quecto container init` can be
//! run from any directory and materializes only missing files.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use sha2::{Digest, Sha256};

pub const STANDARD_ASSET_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StandardAsset {
    pub path: &'static str,
    pub contents: &'static str,
    /// Unix mode requested when the asset is first published. Existing files
    /// are never chmod'ed, preserving user-owned project state.
    pub mode: u32,
}

const ASSETS: &[StandardAsset] = &[
    StandardAsset {
        path: "standard-container/Containerfile",
        contents: include_str!("../../assets/standard-container/Containerfile"),
        mode: 0o644,
    },
    StandardAsset {
        path: "standard-container/config.json",
        contents: include_str!("../../assets/standard-container/config.json"),
        mode: 0o600,
    },
    StandardAsset {
        path: "standard-container/scripts/runtime/create.sh",
        contents: include_str!("../../assets/standard-container/scripts/runtime/create.sh"),
        mode: 0o755,
    },
    StandardAsset {
        path: "standard-container/scripts/runtime/exec.sh",
        contents: include_str!("../../assets/standard-container/scripts/runtime/exec.sh"),
        mode: 0o755,
    },
    StandardAsset {
        path: "standard-container/scripts/runtime/inspect.sh",
        contents: include_str!("../../assets/standard-container/scripts/runtime/inspect.sh"),
        mode: 0o755,
    },
    StandardAsset {
        path: "standard-container/scripts/runtime/kill.sh",
        contents: include_str!("../../assets/standard-container/scripts/runtime/kill.sh"),
        mode: 0o755,
    },
];

/// Return the immutable catalog embedded in this executable.
pub fn standard_assets() -> &'static [StandardAsset] { ASSETS }

/// Stable manifest entries (path, byte length, SHA-256) for diagnostics/tests.
pub fn standard_asset_manifest() -> Vec<(&'static str, usize, String)> {
    ASSETS.iter().map(|asset| {
        let digest = Sha256::digest(asset.contents.as_bytes());
        (asset.path, asset.contents.len(), format!("{digest:x}"))
    }).collect()
}

/// Materialize missing standard assets below `project`.
///
/// Existing regular files and directories are preserved. Symlink targets are
/// rejected rather than followed. `create_new` closes the check/write race.
/// Config placeholders are expanded only for the generated config file.
pub fn materialize_standard_assets(project: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
    let project = project.as_ref();
    materialize_standard_assets_for_root(project, project)
}

/// Materialize into `bundle`, while expanding generated configuration paths
/// relative to the owning project root. The separate roots prevent a bundle
/// nested under `.quecto` from accidentally baking its own internal path.
pub fn materialize_standard_assets_for_root(
    bundle: &Path,
    project_root: &Path,
) -> io::Result<Vec<PathBuf>> {
    ensure_directory_chain(bundle)?;
    let project_root = project_root.canonicalize().unwrap_or_else(|_| project_root.to_path_buf());
    let mut created = Vec::new();
    for (index, asset) in ASSETS.iter().enumerate() {
        let relative = safe_relative_path(asset.path)?;
        let destination = bundle.join(relative);
        reject_symlink_or_existing(&destination)?;
        let parent = destination.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "asset has no parent"))?;
        ensure_directory_chain(parent)?;
        let contents = if asset.path.ends_with("config.json") {
            asset.contents.replace("@PROJECT@", &project_root.to_string_lossy())
        } else { asset.contents.to_owned() };
        if publish_new_atomic(&destination, contents.as_bytes(), asset.mode, index)? {
            created.push(destination);
        }
    }
    Ok(created)
}

fn ensure_directory_chain(path: &Path) -> io::Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("refusing symlink directory: {}", current.display()))),
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("asset parent is not a directory: {}", current.display()))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&current)?,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn reject_symlink_or_existing(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("refusing symlink asset destination: {}", path.display()))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn publish_new_atomic(destination: &Path, bytes: &[u8], mode: u32, index: usize) -> io::Result<bool> {
    let parent = destination.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "asset has no parent"))?;
    let name = destination.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "asset has no name"))?.to_string_lossy();
    let temporary = parent.join(format!(".{name}.quecto-{index}-{}", std::process::id()));
    let mut file = match fs::OpenOptions::new().write(true).create_new(true).open(&temporary) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("temporary asset path already exists: {}", temporary.display()))),
        Err(error) => return Err(error),
    };
    let result = (|| {
        use std::io::Write;
        file.write_all(bytes)?;
        file.sync_all()?;
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))?; }
        match fs::hard_link(&temporary, destination) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error),
        }
    })();
    drop(file);
    let _ = fs::remove_file(&temporary);
    result
}

fn safe_relative_path(path: &str) -> io::Result<PathBuf> {
    let candidate = Path::new(path);
    let components: Vec<_> = candidate.components().collect();
    let valid = !components.is_empty() && components.iter().all(|component| matches!(component, Component::Normal(_)));
    if valid { Ok(components.into_iter().collect()) } else { Err(io::Error::new(io::ErrorKind::InvalidInput, "standard asset path is not relative and safe")) }
}


#[cfg(test)]
#[path = "standard_assets_tests.rs"]
mod standard_assets_tests;
