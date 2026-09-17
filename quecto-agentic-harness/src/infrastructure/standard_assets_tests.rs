use super::standard_assets::*;
use std::fs;
use std::io;
use std::path::PathBuf;

#[test]
fn catalog_is_versioned_and_contains_runtime_assets() {
    assert_eq!(STANDARD_ASSET_VERSION, 1);
    assert!(standard_assets().iter().any(|a| a.path.ends_with("Containerfile")));
    assert_eq!(standard_assets().len(), 6);
    assert!(standard_asset_manifest().iter().all(|(_, size, hash)| *size > 0 && hash.len() == 64));
}
#[test]
fn materialization_is_idempotent_and_expands_config() {
    let dir = tempfile::tempdir().unwrap();
    let first = materialize_standard_assets(dir.path()).unwrap();
    assert_eq!(first.len(), 6);
    let config = fs::read_to_string(dir.path().join("standard-container/config.json")).unwrap();
    assert!(!config.contains("@PROJECT@"));
    assert!(config.contains(dir.path().to_string_lossy().as_ref()));
    assert!(materialize_standard_assets(dir.path()).unwrap().is_empty());
    fs::write(dir.path().join("standard-container/Containerfile"), "user content").unwrap();
    assert!(materialize_standard_assets(dir.path()).unwrap().is_empty());
    assert_eq!(fs::read_to_string(dir.path().join("standard-container/Containerfile")).unwrap(), "user content");
}
#[cfg(unix)]
#[test]
fn materialization_rejects_symlink_destination() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap(); let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("standard-container")).unwrap();
    symlink(outside.path(), root.path().join("standard-container/Containerfile")).unwrap();
    let error = materialize_standard_assets(root.path()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(!outside.path().join("Containerfile").exists());
}
#[test]
fn architecture_guard_rejects_unsafe_paths() {
    for path in ["", "/tmp/escape", "../escape", "a/../escape", "./file"] { assert!(super::safe_relative_path(path).is_err()); }
    assert_eq!(super::safe_relative_path("nested/file").unwrap(), PathBuf::from("nested/file"));
}
