//! #2218 review 4: the store appends only onto a file it can vouch for —
//! one it last wrote or loaded whole, still exactly that length. After a
//! failed write, a foreign record, a replaced or deleted file, or a release,
//! the next write compacts without trusting what it reads back.
use super::super::*;
use super::transcript;
use crate::application::sessions::ports::SessionStore;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use std::os::unix::fs::MetadataExt;
use tempfile::TempDir;

fn key(k: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(k)
}

async fn fresh(tmp: &TempDir, k: &str) -> Vec<String> {
    let fresh = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
    let loaded = fresh.load(&key(k)).await.unwrap().expect("saved");
    loaded.messages.into_iter().map(|m| m.content).collect()
}

fn contents(len: usize) -> Vec<String> {
    transcript(len).into_iter().map(|m| m.content).collect()
}

fn lines(store: &FileSessionStore, k: &str) -> usize {
    std::fs::read_to_string(store.session_path(&key(k)))
        .unwrap()
        .lines()
        .count()
}

fn fail_next_sync(store: &FileSessionStore, k: &str) {
    session_store_write::FAIL_NEXT_SYNC_OF
        .lock()
        .unwrap()
        .push(store.session_path(&key(k)));
}

#[tokio::test]
async fn a_delta_onto_a_shorter_file_rewrites_it() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
    store
        .save_clean_delta(&key("cli:a"), &transcript(3), 0, None)
        .await
        .unwrap();
    let other = FileSessionStore::new(FlatSessionLayout::new(tmp.path().join("o")));
    other
        .save_clean_delta(&key("cli:a"), &transcript(1), 0, None)
        .await
        .unwrap();
    std::fs::copy(
        other.session_path(&key("cli:a")),
        store.session_path(&key("cli:a")),
    )
    .unwrap();
    store
        .save_delta(&key("cli:a"), &transcript(5), 3, None)
        .await
        .unwrap();
    assert_eq!(fresh(&tmp, "cli:a").await, contents(5));
    store
        .save_clean_delta(&key("cli:a"), &transcript(7), 5, None)
        .await
        .unwrap();
    assert_eq!(fresh(&tmp, "cli:a").await, contents(7));
}

#[tokio::test]
async fn a_foreign_terminated_record_is_compacted_over() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
    store
        .save_clean_delta(&key("cli:b"), &transcript(2), 0, None)
        .await
        .unwrap();
    let path = store.session_path(&key("cli:b"));
    let mut data = std::fs::read(&path).unwrap();
    data.extend_from_slice(
        b"{\"type\":\"append\",\"start_index\":2,\"messages\":[{\"role\":\"user\",\"content\":\"landed\"}]}\n",
    );
    std::fs::write(&path, data).unwrap();
    store
        .save_clean_delta(&key("cli:b"), &transcript(4), 2, None)
        .await
        .unwrap();
    assert_eq!(fresh(&tmp, "cli:b").await, contents(4));
    assert_eq!(lines(&store, "cli:b"), 1, "compacted, not appended");
}

#[tokio::test]
async fn a_deleted_or_released_file_is_no_longer_vouched_for() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
    let path = store.session_path(&key("cli:e"));
    store
        .save_clean_delta(&key("cli:e"), &transcript(2), 0, None)
        .await
        .unwrap();
    assert!(store.intact.appendable(&path).await);
    store.save(&Session::new(key("cli:e"))).await.unwrap();
    assert!(
        !store.intact.appendable(&path).await,
        "a delete forgets the file"
    );
    store
        .save_clean_delta(&key("cli:e"), &transcript(2), 0, None)
        .await
        .unwrap();
    assert!(store.intact.appendable(&path).await);
    store.release(&key("cli:e"));
    assert!(
        !store.intact.appendable(&path).await,
        "a release forgets the file"
    );
}

#[tokio::test]
async fn every_save_after_a_failed_sync_compacts() {
    for kind in ["clean", "verified", "full"] {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
        store
            .save_clean_delta(&key("cli:f"), &transcript(2), 0, None)
            .await
            .unwrap();
        fail_next_sync(&store, "cli:f");
        store
            .save_clean_delta(&key("cli:f"), &transcript(3), 2, None)
            .await
            .expect_err("the append landed, then its fsync failed");
        assert_eq!(lines(&store, "cli:f"), 2, "the append landed");
        match kind {
            "clean" => {
                store
                    .save_clean_delta(&key("cli:f"), &transcript(4), 3, None)
                    .await
            }
            "verified" => {
                store
                    .save_delta(&key("cli:f"), &transcript(4), 3, None)
                    .await
            }
            _ => {
                let mut session = Session::new(key("cli:f"));
                session.messages = transcript(4);
                store.save(&session).await
            }
        }
        .unwrap();
        assert_eq!(
            lines(&store, "cli:f"),
            1,
            "{kind}: compacted, never appended"
        );
        assert_eq!(fresh(&tmp, "cli:f").await, contents(4));
    }
}

/// The transcript's bytes with its first message's text changed in place:
/// a file of exactly the same length the store did not write.
fn same_length_foreign(bytes: &[u8]) -> Vec<u8> {
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("task 0"));
    text.replacen("task 0", "tosk 0", 1).into_bytes()
}

#[tokio::test]
async fn a_file_recreated_after_a_delete_is_not_vouched_for_even_at_the_same_length() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
    store
        .save_clean_delta(&key("cli:g"), &transcript(2), 0, None)
        .await
        .unwrap();
    let path = store.session_path(&key("cli:g"));
    let saved = std::fs::read(&path).unwrap();
    store.save(&Session::new(key("cli:g"))).await.unwrap();
    assert!(!path.exists(), "the empty save deleted the transcript");
    std::fs::write(&path, same_length_foreign(&saved)).unwrap();
    store
        .save_clean_delta(&key("cli:g"), &transcript(4), 2, None)
        .await
        .unwrap();
    assert_eq!(fresh(&tmp, "cli:g").await, contents(4));
}

/// Rewrite `path` in place with same-length foreign bytes and restore its
/// modification time: a file whose identity the store cannot tell apart.
fn indistinguishable_rewrite(path: &std::path::Path) {
    let before = std::fs::metadata(path).unwrap();
    let foreign = same_length_foreign(&std::fs::read(path).unwrap());
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    std::io::Write::write_all(&mut &file, &foreign).unwrap();
    file.set_modified(before.modified().unwrap()).unwrap();
}

/// A write that fails — even before it writes a byte — makes the store
/// forget the file: nothing proves it is still the file it left, so the
/// next save compacts (#2218 reviews 3 and 4).
#[tokio::test]
async fn a_write_that_failed_before_landing_still_forgets_the_file() {
    for kind in ["clean", "verified"] {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
        store
            .save_clean_delta(&key("cli:h"), &transcript(2), 0, None)
            .await
            .unwrap();
        store.fail_next_write(&key("cli:h"));
        let failed = match kind {
            "verified" => {
                store
                    .save_delta(&key("cli:h"), &transcript(4), 2, None)
                    .await
            }
            _ => {
                store
                    .save_clean_delta(&key("cli:h"), &transcript(4), 2, None)
                    .await
            }
        };
        failed.expect_err("the write failed before landing");
        assert_eq!(lines(&store, "cli:h"), 1, "{kind}: nothing landed");
        indistinguishable_rewrite(&store.session_path(&key("cli:h")));
        store
            .save_clean_delta(&key("cli:h"), &transcript(4), 2, None)
            .await
            .unwrap();
        assert_eq!(
            fresh(&tmp, "cli:h").await,
            contents(4),
            "{kind}: compacted over it"
        );
    }
}

/// Another process swaps in different, valid JSONL of exactly the same
/// length — by a rename (a new inode) or an in-place rewrite (a new
/// modification time): the store no longer vouches for the file, and the
/// next clean delta compacts instead of appending onto the foreign prefix
/// (#2235 review).
#[tokio::test]
async fn a_same_length_foreign_file_is_compacted_over_however_it_was_swapped_in() {
    for swap in ["rename", "in-place"] {
        let tmp = TempDir::new().unwrap();
        let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
        store
            .save_clean_delta(&key("cli:i"), &transcript(2), 0, None)
            .await
            .unwrap();
        let path = store.session_path(&key("cli:i"));
        let foreign = same_length_foreign(&std::fs::read(&path).unwrap());
        let original = std::fs::metadata(&path).unwrap();
        match swap {
            // Only the inode differs: same length, same modification time.
            "rename" => {
                let staged = path.with_extension("foreign");
                std::fs::write(&staged, &foreign).unwrap();
                let file = std::fs::File::options().write(true).open(&staged).unwrap();
                file.set_modified(original.modified().unwrap()).unwrap();
                std::fs::rename(&staged, &path).unwrap();
            }
            // Only the modification time differs: same inode, same length.
            _ => {
                let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
                std::io::Write::write_all(&mut &file, &foreign).unwrap();
                let later = original.modified().unwrap() + std::time::Duration::from_secs(2);
                file.set_modified(later).unwrap();
            }
        }
        let swapped = std::fs::metadata(&path).unwrap();
        assert_eq!(
            (
                swapped.ino() != original.ino(),
                swapped.modified().unwrap() != original.modified().unwrap()
            ),
            (swap == "rename", swap != "rename"),
            "{swap}: exactly one identity field differs"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().len() as usize,
            foreign.len()
        );
        store
            .save_clean_delta(&key("cli:i"), &transcript(4), 2, None)
            .await
            .unwrap();
        assert_eq!(
            fresh(&tmp, "cli:i").await,
            contents(4),
            "{swap}: compacted over it"
        );
    }
}

#[tokio::test]
async fn the_store_vouches_for_the_file_a_compaction_renamed_into_place() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
    store
        .save_clean_delta(&key("cli:j"), &transcript(2), 0, None)
        .await
        .unwrap();
    let path = store.session_path(&key("cli:j"));
    assert!(
        store.intact.appendable(&path).await,
        "the new inode was recorded"
    );
    store
        .save_clean_delta(&key("cli:j"), &transcript(4), 2, None)
        .await
        .unwrap();
    assert_eq!(
        lines(&store, "cli:j"),
        2,
        "a vouched-for file is appended to"
    );
}

/// A file swapped while it was being loaded is not vouched for: the load
/// records the file only if it is the one the read began on (#2235 review).
#[tokio::test]
async fn a_file_swapped_during_its_load_is_not_vouched_for() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));
    store
        .save_clean_delta(&key("cli:k"), &transcript(2), 0, None)
        .await
        .unwrap();
    let path = store.session_path(&key("cli:k"));
    let (data, before) = store.intact.read(&path).await.unwrap();
    let staged = path.with_extension("foreign");
    std::fs::write(&staged, same_length_foreign(data.as_bytes())).unwrap();
    std::fs::rename(&staged, &path).unwrap();
    store
        .intact
        .observe_read(&path, before, data.len(), true)
        .await;
    assert!(
        !store.intact.appendable(&path).await,
        "the read began on another file"
    );
    let (data, before) = store.intact.read(&path).await.unwrap();
    store
        .intact
        .observe_read(&path, before, data.len(), true)
        .await;
    assert!(
        store.intact.appendable(&path).await,
        "an unchanged file is vouched for"
    );
}
