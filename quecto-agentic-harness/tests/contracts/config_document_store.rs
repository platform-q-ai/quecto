//! Contract for the `ConfigDocumentStore` port (#2024): the adapter reports
//! absence only when there is no directory entry at all; anything present
//! is either the file's bytes or a named error. Selection relies on this
//! to never fall back past a present-but-broken config file.
use std::sync::Arc;
use tempfile::TempDir;

use quecto::application::configuration::ports::ConfigDocumentStore;
use quecto::infrastructure::config::loaders::FilesystemConfigDocumentStore;

fn under_test() -> Arc<dyn ConfigDocumentStore> {
    Arc::new(FilesystemConfigDocumentStore)
}

#[test]
fn only_a_missing_entry_reads_as_none() {
    let dir = TempDir::new().unwrap();
    let store = under_test();
    let path = dir.path().join("config.json");
    assert_eq!(store.read(&path).unwrap(), None);
    assert!(!store.is_present(&path));

    std::fs::write(&path, "{}").unwrap();
    assert_eq!(store.read(&path).unwrap(), Some(b"{}".to_vec()));
    assert!(store.is_present(&path));
}

#[test]
fn a_symlink_to_a_regular_file_reads_and_a_dangling_one_is_an_error() {
    let dir = TempDir::new().unwrap();
    let store = under_test();
    let target = dir.path().join("shared.json");
    std::fs::write(&target, "{}").unwrap();
    let good = dir.path().join("good.json");
    std::os::unix::fs::symlink(&target, &good).unwrap();
    assert_eq!(store.read(&good).unwrap(), Some(b"{}".to_vec()));

    let dangling = dir.path().join("dangling.json");
    std::os::unix::fs::symlink(dir.path().join("missing.json"), &dangling).unwrap();
    assert!(
        store.read(&dangling).is_err(),
        "a dangling link is present, never absent"
    );
    assert!(store.is_present(&dangling));
}

#[test]
fn a_directory_is_an_error_naming_the_shape() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    std::fs::create_dir(&path).unwrap();
    let error = under_test().read(&path).unwrap_err();
    assert!(error.contains("not a regular file"), "{error}");
}
