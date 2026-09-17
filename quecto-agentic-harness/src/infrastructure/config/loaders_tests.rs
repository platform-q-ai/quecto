use super::*;
use tempfile::TempDir;

#[test]
fn absent_files_read_as_none_and_present_ones_as_bytes() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    let store = FilesystemConfigDocumentStore;
    assert_eq!(store.read(&path).unwrap(), None);
    assert!(!store.is_present(&path));
    std::fs::write(&path, "{}").unwrap();
    assert_eq!(store.read(&path).unwrap(), Some(b"{}".to_vec()));
    assert!(store.is_present(&path));
}

#[test]
fn a_directory_or_a_dangling_symlink_is_an_error_not_absence() {
    let dir = TempDir::new().unwrap();
    let store = FilesystemConfigDocumentStore;
    let directory = dir.path().join("config.json");
    std::fs::create_dir(&directory).unwrap();
    let error = store.read(&directory).unwrap_err();
    assert!(error.contains("not a regular file"), "{error}");
    assert!(store.is_present(&directory));

    let dangling = dir.path().join("dangling.json");
    std::os::unix::fs::symlink(dir.path().join("missing.json"), &dangling).unwrap();
    let error = store.read(&dangling).unwrap_err();
    assert!(error.contains("cannot be read"), "{error}");
    assert!(
        store.is_present(&dangling),
        "the entry exists even though its target does not"
    );

    let unsearchable = dir.path().join("locked");
    std::fs::create_dir(&unsearchable).unwrap();
    let inside = unsearchable.join("config.json");
    std::fs::write(&inside, "{}").unwrap();
    std::fs::set_permissions(
        &unsearchable,
        std::os::unix::fs::PermissionsExt::from_mode(0o000),
    )
    .unwrap();
    let outcome = store.read(&inside);
    std::fs::set_permissions(
        &unsearchable,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    if nix_is_root() {
        assert!(outcome.is_ok());
    } else {
        assert!(
            outcome.is_err(),
            "an unsearchable parent is an error, not absence"
        );
    }
}

fn nix_is_root() -> bool {
    std::fs::metadata("/proc/self")
        .map(|m| {
            use std::os::unix::fs::MetadataExt;
            m.uid() == 0
        })
        .unwrap_or(false)
}
