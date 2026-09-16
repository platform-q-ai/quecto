use super::*;
use tempfile::TempDir;

fn running_as_root() -> bool {
    // SAFETY: geteuid has no preconditions and only reads the effective uid.
    unsafe { libc::geteuid() == 0 }
}

#[test]
fn absent_entry_is_absent() {
    let dir = TempDir::new().unwrap();
    assert_eq!(
        FilesystemLocalConfigProbe.probe(&dir.path().join("config.json")),
        LocalConfigPresence::Absent
    );
}

#[test]
fn regular_file_is_usable() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, "{}").unwrap();
    assert_eq!(
        FilesystemLocalConfigProbe.probe(&path),
        LocalConfigPresence::RegularFile
    );
}

#[test]
fn symlink_to_a_regular_file_is_usable() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("shared.json");
    std::fs::write(&target, "{}").unwrap();
    let link = dir.path().join("config.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(
        FilesystemLocalConfigProbe.probe(&link),
        LocalConfigPresence::RegularFile
    );
}

#[test]
fn directory_is_not_a_regular_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    std::fs::create_dir(&path).unwrap();
    assert_eq!(
        FilesystemLocalConfigProbe.probe(&path),
        LocalConfigPresence::NotRegularFile
    );
}

#[test]
fn dangling_symlink_is_present_but_unreadable() {
    let dir = TempDir::new().unwrap();
    let link = dir.path().join("config.json");
    std::os::unix::fs::symlink(dir.path().join("missing.json"), &link).unwrap();
    match FilesystemLocalConfigProbe.probe(&link) {
        LocalConfigPresence::Unreadable(reason) => {
            assert!(reason.contains("No such file"), "{reason}")
        }
        other => panic!("expected Unreadable, got {other:?}"),
    }
}

#[test]
fn entry_in_an_unsearchable_directory_is_unreadable() {
    use std::os::unix::fs::PermissionsExt;
    if running_as_root() {
        return; // root ignores mode bits; the dangling-symlink case covers the branch.
    }
    let parent = TempDir::new().unwrap();
    let locked = parent.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::write(locked.join("config.json"), "{}").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let presence = FilesystemLocalConfigProbe.probe(&locked.join("config.json"));
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    match presence {
        LocalConfigPresence::Unreadable(reason) => {
            assert!(reason.contains("ermission denied"), "{reason}")
        }
        other => panic!("expected Unreadable, got {other:?}"),
    }
}
