//! Contract for the `LocalConfigProbe` port (#1966): the adapter reports
//! absence only when there is no directory entry at all; anything present
//! is either a usable regular file (a symlink to one included) or a named
//! rejection. Selection policy relies on this to never fall back past a
//! present-but-broken local config.
use std::sync::Arc;
use tempfile::TempDir;

use quecto::application::configuration::ports::{LocalConfigPresence, LocalConfigProbe};
use quecto::infrastructure::local_config_probe::FilesystemLocalConfigProbe;

fn under_test() -> Arc<dyn LocalConfigProbe> {
    Arc::new(FilesystemLocalConfigProbe)
}

#[test]
fn only_a_missing_entry_is_absent() {
    let dir = TempDir::new().unwrap();
    let probe = under_test();
    let path = dir.path().join("config.json");
    assert_eq!(probe.probe(&path), LocalConfigPresence::Absent);

    std::fs::write(&path, "{}").unwrap();
    assert_eq!(probe.probe(&path), LocalConfigPresence::RegularFile);
}

#[test]
fn a_symlink_to_a_regular_file_is_usable_and_a_dangling_one_is_not() {
    let dir = TempDir::new().unwrap();
    let probe = under_test();
    let target = dir.path().join("shared.json");
    std::fs::write(&target, "{}").unwrap();
    let good = dir.path().join("good.json");
    std::os::unix::fs::symlink(&target, &good).unwrap();
    assert_eq!(probe.probe(&good), LocalConfigPresence::RegularFile);

    let dangling = dir.path().join("dangling.json");
    std::os::unix::fs::symlink(dir.path().join("missing.json"), &dangling).unwrap();
    assert!(
        matches!(probe.probe(&dangling), LocalConfigPresence::Unreadable(_)),
        "a dangling link is present, never absent"
    );
}

#[test]
fn a_directory_is_not_a_regular_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    std::fs::create_dir(&path).unwrap();
    assert_eq!(
        under_test().probe(&path),
        LocalConfigPresence::NotRegularFile
    );
}
