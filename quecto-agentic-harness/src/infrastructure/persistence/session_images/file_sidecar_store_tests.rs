//! The file adapter's own rules (#2424): owner-only files written once and
//! repaired when their content is not their name's, nothing read that is
//! not a regular file within the cap (a link or a FIFO never blocks), and
//! only the store's own names ever removed.
use std::collections::BTreeSet;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::super::sidecar_store::{ImageSidecarStore, SidecarRead};
use super::FileImageSidecarStore;
use crate::domain::conversation::value_objects::stored_images::{
    ImageRef, MAX_STORED_IMAGE_TEXT, VerifiedText, sha256_hex,
};
use crate::domain::sessions::entities::session_identity::SessionIdentity;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;

fn id() -> SessionIdentity {
    SessionIdentity::from_persisted_key("cli:sidecars")
}

fn reference(text: &str) -> ImageRef {
    ImageRef {
        sha256: sha256_hex(text.as_bytes()),
        mime_type: "image/png".into(),
    }
}

fn store(dir: &Path) -> (FileImageSidecarStore, PathBuf) {
    let layout = FlatSessionLayout::new(dir);
    let images = layout.image_dir(&id());
    (FileImageSidecarStore::new(layout), images)
}

#[tokio::test]
async fn a_sidecar_is_owner_only_holds_the_text_and_is_written_once() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, dir) = store(tmp.path());
    let text = "iVBORw0KGgo=";
    store.put(&id(), &reference(text), text).await.unwrap();
    let path = dir.join(&reference(text).sha256);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    let first = std::fs::metadata(&path).unwrap();
    assert_eq!(first.permissions().mode() & 0o777, 0o600);
    store.put(&id(), &reference(text), text).await.unwrap();
    let second = std::fs::metadata(&path).unwrap();
    assert_eq!(first.ino(), second.ino(), "an image already stored is kept");
}

#[tokio::test]
async fn a_sidecar_whose_content_is_not_its_name_is_written_again() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, dir) = store(tmp.path());
    let text = "iVBORw0KGgoAAAA=";
    store.put(&id(), &reference(text), text).await.unwrap();
    let path = dir.join(&reference(text).sha256);
    // The same length, other bytes: only its content says it is wrong.
    std::fs::write(&path, "iVBORw0KGgoBBBB=").unwrap();
    assert_eq!(
        store.get(&id(), &reference(text).sha256).await.unwrap(),
        SidecarRead::Corrupt
    );
    store.put(&id(), &reference(text), text).await.unwrap();
    assert_eq!(
        store.get(&id(), &reference(text).sha256).await.unwrap(),
        SidecarRead::Found(VerifiedText::of(text.to_string()))
    );
}

/// The CI failure (#2424): a same-length rewrite within the timestamp tick
/// of the store's own write leaves the file's stamp exactly as the store
/// recorded it. A stamp that recent is never trusted unread, so an unchanged
/// stat is not taken for a verified file.
#[tokio::test]
async fn a_stamp_taken_within_the_tick_of_a_change_is_not_trusted() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, dir) = store(tmp.path());
    let text = "iVBORw0KGgoAAAA=";
    store.put(&id(), &reference(text), text).await.unwrap();
    let path = dir.join(&reference(text).sha256);
    let unchanged = std::fs::metadata(&path).unwrap();
    assert!(
        !store.still_verified(&path, &unchanged),
        "the same stamp, but too recent to vouch for the content"
    );
    let (stamp, at) = store.known()[&path];
    assert!(!stamp.settled_before(at));
    assert!(stamp.settled_before(at + super::RACY_WINDOW_NS + 1_000_000_000));
}

/// A read that finds a sidecar corrupt forgets it, whatever its times say:
/// the next store of its image reads it again and repairs it.
#[tokio::test]
async fn a_sidecar_found_corrupt_is_trusted_no_more() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, dir) = store(tmp.path());
    let text = "iVBORw0KGgoAAAA=";
    store.put(&id(), &reference(text), text).await.unwrap();
    let path = dir.join(&reference(text).sha256);
    assert!(store.known().contains_key(&path));
    std::fs::write(&path, "iVBORw0KGgoBBBB=").unwrap();
    let read = store.get(&id(), &reference(text).sha256).await.unwrap();
    assert_eq!(read, SidecarRead::Corrupt);
    assert!(!store.known().contains_key(&path), "forgotten");
}

#[tokio::test]
async fn a_link_a_fifo_or_an_oversized_file_is_never_read() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, dir) = store(tmp.path());
    std::fs::create_dir_all(&dir).unwrap();
    let target = tmp.path().join("elsewhere");
    std::fs::write(&target, "linked").unwrap();
    let linked = reference("linked").sha256;
    std::os::unix::fs::symlink(&target, dir.join(&linked)).unwrap();
    assert_eq!(
        store.get(&id(), &linked).await.unwrap(),
        SidecarRead::Corrupt
    );

    let fifo = reference("fifo").sha256;
    let made = std::process::Command::new("mkfifo")
        .arg(dir.join(&fifo))
        .status()
        .unwrap();
    assert!(made.success());
    let session = id();
    let read = tokio::time::timeout(Duration::from_secs(5), store.get(&session, &fifo));
    assert_eq!(
        read.await.expect("never blocks").unwrap(),
        SidecarRead::Corrupt
    );

    let huge = reference("huge").sha256;
    let file = std::fs::File::create(dir.join(&huge)).unwrap();
    file.set_len(MAX_STORED_IMAGE_TEXT as u64 + 1).unwrap();
    assert_eq!(store.get(&id(), &huge).await.unwrap(), SidecarRead::Corrupt);
}

#[tokio::test]
async fn only_the_stores_own_names_are_ever_removed_and_fresh_writes_are_spared() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, dir) = store(tmp.path());
    let text = "R0lGODlh";
    store.put(&id(), &reference(text), text).await.unwrap();
    std::fs::write(dir.join("notes.txt"), b"not a sidecar").unwrap();
    std::fs::write(dir.join(".fresh.1234.tmp"), b"in flight").unwrap();
    let stale = dir.join(".stale.5678.tmp");
    std::fs::write(&stale, b"left by a crash").unwrap();
    let old = SystemTime::now() - Duration::from_secs(120);
    std::fs::File::options()
        .write(true)
        .open(&stale)
        .unwrap()
        .set_modified(old)
        .unwrap();

    store.retain_only(&id(), &BTreeSet::new()).await.unwrap();
    assert!(
        !dir.join(reference(text).sha256).exists(),
        "garbage is collected"
    );
    assert!(!stale.exists(), "a write a minute old is over");
    assert!(
        dir.join(".fresh.1234.tmp").exists(),
        "a write may be in flight"
    );
    assert!(dir.join("notes.txt").exists());

    store.remove_all(&id()).await.unwrap();
    assert!(!dir.join(".fresh.1234.tmp").exists(), "the session is gone");
    assert!(
        dir.join("notes.txt").exists(),
        "what the store never wrote stays"
    );
    std::fs::remove_file(dir.join("notes.txt")).unwrap();
    store.remove_all(&id()).await.unwrap();
    assert!(!dir.exists(), "an emptied directory goes");
}
