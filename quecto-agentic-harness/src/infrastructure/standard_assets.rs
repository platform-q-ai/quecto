//! Embedded, versioned standard project assets.
//!
//! Assets are shipped with the binary and can be copied into a project without
//! ever escaping the caller-provided project directory.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

pub const STANDARD_ASSET_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StandardAsset {
    pub path: &'static str,
    pub contents: &'static str,
}

const ASSETS: &[StandardAsset] = &[StandardAsset {
    path: "AGENTS.md",
    contents: include_str!("../../assets/AGENTS.md"),
}];

/// Return the immutable catalog embedded in this executable.
pub fn standard_assets() -> &'static [StandardAsset] {
    ASSETS
}

/// Materialize missing standard assets below `project`. Existing files are
/// deliberately preserved so initialization cannot destroy user work.
pub fn materialize_standard_assets(project: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
    let project = project.as_ref();
    let mut created = Vec::new();
    for asset in ASSETS {
        let relative = safe_relative_path(asset.path)?;
        let destination = project.join(relative);
        if destination.exists() {
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        // create_new closes the TOCTOU gap between existence check and write.
        match fs::OpenOptions::new().write(true).create_new(true).open(&destination) {
            Ok(mut file) => {
                use std::io::Write;
                if let Err(error) = file.write_all(asset.contents.as_bytes()) {
                    let _ = fs::remove_file(&destination);
                    return Err(error);
                }
                created.push(destination);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Ok(created)
}

fn safe_relative_path(path: &str) -> io::Result<PathBuf> {
    let candidate = Path::new(path);
    let components: Vec<_> = candidate.components().collect();
    let valid = !components.is_empty()
        && components.iter().all(|component| matches!(component, Component::Normal(_)));
    if valid {
        Ok(components.into_iter().collect())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "standard asset path is not relative and safe"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_versioned_and_embedded() {
        assert_eq!(STANDARD_ASSET_VERSION, 1);
        assert!(standard_assets().iter().any(|a| a.path == "AGENTS.md"));
    }

    #[test]
    fn materialization_is_idempotent_and_preserves_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let first = materialize_standard_assets(dir.path()).unwrap();
        assert_eq!(first.len(), 1);
        let original = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(materialize_standard_assets(dir.path()).unwrap().is_empty());
        fs::write(dir.path().join("AGENTS.md"), "user content").unwrap();
        assert!(materialize_standard_assets(dir.path()).unwrap().is_empty());
        assert_eq!(fs::read_to_string(dir.path().join("AGENTS.md")).unwrap(), "user content");
        assert!(!original.is_empty());
    }

    #[test]
    fn architecture_guard_rejects_every_non_normal_asset_path() {
        for path in ["", "/tmp/escape", "../escape", "a/../escape", "./file"] {
            assert!(safe_relative_path(path).is_err(), "accepted unsafe asset path: {path:?}");
        }
        assert_eq!(safe_relative_path("nested/file").unwrap(), PathBuf::from("nested/file"));
    }

    #[test]
    fn materialization_creates_missing_project_directories() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("new/project");
        let created = materialize_standard_assets(&project).unwrap();
        assert_eq!(created, vec![project.join("AGENTS.md")]);
        assert!(project.join("AGENTS.md").is_file());
    }
}
