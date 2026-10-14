use super::*;
use tempfile::TempDir;

#[test]
fn absent_files_read_as_none_and_present_ones_as_bytes() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    let store = FilesystemConfigDocumentStore;
    assert_eq!(store.read(&path).unwrap(), None);
    std::fs::write(&path, "{}").unwrap();
    assert_eq!(store.read(&path).unwrap(), Some(b"{}".to_vec()));
}

#[test]
fn a_directory_or_a_dangling_symlink_is_an_error_not_absence() {
    let dir = TempDir::new().unwrap();
    let store = FilesystemConfigDocumentStore;
    let directory = dir.path().join("config.json");
    std::fs::create_dir(&directory).unwrap();
    let error = store.read(&directory).unwrap_err();
    assert!(error.contains("not a regular file"), "{error}");

    let dangling = dir.path().join("dangling.json");
    std::os::unix::fs::symlink(dir.path().join("missing.json"), &dangling).unwrap();
    let error = store.read(&dangling).unwrap_err();
    assert!(error.contains("cannot be read"), "{error}");

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

#[test]
fn the_overlay_policy_checks_exactly_the_entries_below_the_working_directory() {
    let dir = TempDir::new().unwrap();
    let store = FilesystemConfigDocumentStore;
    let cwd = dir.path().join("repo");
    let overlay = cwd.join(OVERLAY_RELATIVE_PATH);
    std::fs::create_dir_all(overlay.parent().unwrap()).unwrap();
    std::fs::write(&overlay, "{}").unwrap();
    assert_eq!(
        store.read_overlay(&overlay).unwrap(),
        OverlayDocument::Present(b"{}".to_vec())
    );
    // A link *above* the working directory is the caller's business
    // (selection canonicalises the working directory), not the overlay's.
    let parent_link = dir.path().join("repo-link");
    std::os::unix::fs::symlink(&cwd, &parent_link).unwrap();
    assert_eq!(
        store
            .read_overlay(&parent_link.join(OVERLAY_RELATIVE_PATH))
            .unwrap(),
        OverlayDocument::Present(b"{}".to_vec())
    );
    // Replacing the `.quecto` directory with a link to a directory that
    // holds the same file is refused, naming the link.
    let elsewhere = dir.path().join("elsewhere");
    std::fs::rename(overlay.parent().unwrap(), &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, overlay.parent().unwrap()).unwrap();
    let OverlayDocument::Refused { reason } = store.read_overlay(&overlay).unwrap() else {
        panic!("a linked .quecto is refused");
    };
    assert!(
        reason.starts_with(&format!(
            "{} is a symbolic link",
            overlay.parent().unwrap().display()
        )),
        "{reason}"
    );
    assert!(reason.contains("replace the link with a copy"), "{reason}");
}

#[test]
fn an_overlay_path_too_short_to_hold_the_layout_is_an_error() {
    let store = FilesystemConfigDocumentStore;
    assert!(store.read_overlay(Path::new("/")).is_err());
}

fn nix_is_root() -> bool {
    std::fs::metadata("/proc/self")
        .map(|m| {
            use std::os::unix::fs::MetadataExt;
            m.uid() == 0
        })
        .unwrap_or(false)
}

/// Review round 3 LOW-3: the overlay file is opened without following a
/// final link and without blocking, and the handle is what is checked: a
/// link swapped in after the stretch check is refused as a link, a FIFO is
/// not a regular file, and neither is followed or waited on.
#[test]
fn the_overlay_open_refuses_a_link_and_a_fifo_by_the_handle() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("target.json");
    std::fs::write(&target, "{}").unwrap();
    let link = dir.path().join("config.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    match open_overlay_file(&link).unwrap() {
        OverlayDocument::Refused { reason } => {
            assert!(reason.contains("symbolic link"), "{reason}")
        }
        other => panic!("a link is refused, got {other:?}"),
    }
    std::fs::remove_file(&link).unwrap();
    let status = std::process::Command::new("mkfifo")
        .arg(&link)
        .status()
        .unwrap();
    assert!(status.success());
    let error = open_overlay_file(&link).unwrap_err();
    assert!(error.contains("not a regular file"), "{error}");
    std::fs::remove_file(&link).unwrap();
    assert_eq!(open_overlay_file(&link).unwrap(), OverlayDocument::Absent);
    std::fs::write(&link, "{}").unwrap();
    assert_eq!(
        open_overlay_file(&link).unwrap(),
        OverlayDocument::Present(b"{}".to_vec())
    );
}
