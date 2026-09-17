use super::*;
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

#[cfg(unix)]
#[test]
fn materialization_rejects_symlink_parent_without_touching_target() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("standard-container")).unwrap();
    symlink(outside.path(), root.path().join("standard-container/scripts")).unwrap();
    let error = materialize_standard_assets(root.path()).unwrap_err();
    assert!(matches!(error.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENOTDIR)), "{error}");
    assert!(!outside.path().join("runtime/create.sh").exists());
}

#[cfg(unix)]
#[test]
fn config_path_is_escaped_as_json_not_interpolated() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project\"\\line\nfeed");
    fs::create_dir(&project).unwrap();
    let bundle = project.join(".quecto/containers");
    materialize_standard_assets_for_root(&bundle, &project).unwrap();
    let config = fs::read_to_string(bundle.join("standard-container/config.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&config).expect("expanded config remains JSON");
    let create = parsed["container_configs"]["standard"]["create"][0].as_str().unwrap();
    assert_eq!(create, format!("{}/scripts/runtime/create.sh", project.display()));
    assert!(config.contains("\\\""), "quote must be escaped in JSON: {config}");
}

#[cfg(unix)]
#[test]
fn concurrent_publishers_preserve_one_complete_asset_set() {
    use std::sync::Arc;
    let root = tempfile::tempdir().unwrap();
    let root = Arc::new(root);
    let mut workers = Vec::new();
    for _ in 0..8 {
        let root = Arc::clone(&root);
        workers.push(std::thread::spawn(move || {
            super::materialize_standard_assets(root.path()).unwrap()
        }));
    }
    let created: Vec<_> = workers.into_iter().map(|worker| worker.join().unwrap()).collect();
    assert_eq!(created.iter().map(Vec::len).sum::<usize>(), 6);
    for asset in super::standard_assets() {
        let path = root.path().join(asset.path);
        assert!(path.is_file(), "missing published asset: {}", path.display());
        assert_eq!(fs::read(path).unwrap().len(), if asset.path.ends_with("config.json") {
            super::expanded_contents(asset, root.path()).unwrap().len()
        } else { asset.contents.len() });
    }
}

#[test]
fn architecture_guard_rejects_unsafe_paths() {
    for path in ["", "/tmp/escape", "../escape", "a/../escape", "./file"] { assert!(super::safe_relative_path(path).is_err()); }
    assert_eq!(super::safe_relative_path("nested/file").unwrap(), PathBuf::from("nested/file"));
}
