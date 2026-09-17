//! Contract for the `ConfigDocumentStore` port (#2024): the adapter reports
//! absence only when there is no directory entry at all; anything present
//! is either the file's bytes or a named error. Selection relies on this
//! to never fall back past a present-but-broken config file. The overlay
//! read adds the one overlay policy: a symbolic link anywhere below the
//! working directory is refused, never read and never written through.
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

use quecto::application::configuration::ports::{ConfigDocumentStore, OverlayDocument};
use quecto::infrastructure::config::loaders::FilesystemConfigDocumentStore;

fn under_test() -> Arc<dyn ConfigDocumentStore> {
    Arc::new(FilesystemConfigDocumentStore)
}

fn refusal(document: OverlayDocument) -> String {
    match document {
        OverlayDocument::Refused { reason } => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn only_a_missing_entry_reads_as_none() {
    let dir = TempDir::new().unwrap();
    let store = under_test();
    let path = dir.path().join("config.json");
    assert_eq!(store.read(&path).unwrap(), None);

    std::fs::write(&path, "{}").unwrap();
    assert_eq!(store.read(&path).unwrap(), Some(b"{}".to_vec()));
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
}

#[test]
fn a_directory_is_an_error_naming_the_shape() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    std::fs::create_dir(&path).unwrap();
    let error = under_test().read(&path).unwrap_err();
    assert!(error.contains("not a regular file"), "{error}");
}

#[test]
fn an_overlay_reads_as_absent_bytes_or_a_named_error() {
    let dir = TempDir::new().unwrap();
    let store = under_test();
    let overlay = dir.path().join("repo").join(".quecto").join("config.json");
    assert_eq!(
        store.read_overlay(&overlay).unwrap(),
        OverlayDocument::Absent
    );
    std::fs::create_dir_all(overlay.parent().unwrap()).unwrap();
    assert_eq!(
        store.read_overlay(&overlay).unwrap(),
        OverlayDocument::Absent,
        "an empty .quecto directory holds no overlay"
    );
    std::fs::write(&overlay, "{}").unwrap();
    assert_eq!(
        store.read_overlay(&overlay).unwrap(),
        OverlayDocument::Present(b"{}".to_vec())
    );
    std::fs::remove_file(&overlay).unwrap();
    std::fs::create_dir(&overlay).unwrap();
    let error = store.read_overlay(&overlay).unwrap_err();
    assert!(error.contains("not a regular file"), "{error}");
}

#[test]
fn an_overlay_behind_a_symbolic_link_at_any_level_below_the_working_directory_is_refused() {
    let dir = TempDir::new().unwrap();
    let store = under_test();
    let other = dir.path().join("other").join(".quecto");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("config.json"), "{\"borrowed\":true}").unwrap();

    // The file itself is a link.
    let repo_a = dir.path().join("repo-a").join(".quecto");
    std::fs::create_dir_all(&repo_a).unwrap();
    let file_link = repo_a.join("config.json");
    std::os::unix::fs::symlink(other.join("config.json"), &file_link).unwrap();
    let reason = refusal(store.read_overlay(&file_link).unwrap());
    assert!(reason.contains("symbolic link"), "{reason}");
    assert!(
        reason.contains(&file_link.display().to_string()),
        "{reason}"
    );

    // The `.quecto` directory is a link, the file behind it regular.
    let repo_b = dir.path().join("repo-b");
    std::fs::create_dir_all(&repo_b).unwrap();
    let dir_link = repo_b.join(".quecto");
    std::os::unix::fs::symlink(&other, &dir_link).unwrap();
    let reason = refusal(store.read_overlay(&dir_link.join("config.json")).unwrap());
    assert!(reason.contains("symbolic link"), "{reason}");
    assert!(reason.contains(&dir_link.display().to_string()), "{reason}");

    // A linked `.quecto` with no file behind it is still refused, not
    // absent: a write would otherwise create the file in the target.
    let repo_c = dir.path().join("repo-c");
    std::fs::create_dir_all(&repo_c).unwrap();
    let empty = dir.path().join("empty-quecto");
    std::fs::create_dir_all(&empty).unwrap();
    std::os::unix::fs::symlink(&empty, repo_c.join(".quecto")).unwrap();
    refusal(
        store
            .read_overlay(&repo_c.join(".quecto").join("config.json"))
            .unwrap(),
    );

    // A dangling link is refused as a link, before readability matters.
    let repo_d = dir.path().join("repo-d").join(".quecto");
    std::fs::create_dir_all(&repo_d).unwrap();
    std::os::unix::fs::symlink(dir.path().join("missing.json"), repo_d.join("config.json"))
        .unwrap();
    refusal(store.read_overlay(&repo_d.join("config.json")).unwrap());

    // The working directory itself may be reached through a link: only the
    // entries below it are the overlay's.
    let via_link = dir.path().join("cwd-link");
    std::os::unix::fs::symlink(dir.path().join("other"), &via_link).unwrap();
    assert_eq!(
        store
            .read_overlay(&via_link.join(Path::new(".quecto").join("config.json")))
            .unwrap(),
        OverlayDocument::Present(b"{\"borrowed\":true}".to_vec())
    );
}
