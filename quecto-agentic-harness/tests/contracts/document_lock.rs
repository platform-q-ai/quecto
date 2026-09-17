//! Contract for the `DocumentLock` port (#2024): the hold a writer hands
//! out serialises read → patch → write cycles — a second holder of the
//! same document waits until the first is dropped, whichever thread or
//! process holds it — and taking it creates nothing where the document
//! lives, so a refused write of a not-yet-existing document leaves its
//! directory as it was.
use std::sync::Arc;
use tempfile::TempDir;

use quecto::application::configuration::ports::{ConfigDocumentWriter, DocumentLock};
use quecto::infrastructure::config::writer::JsonDocumentWriter;

fn under_test(base_dir: &std::path::Path) -> Arc<dyn ConfigDocumentWriter> {
    Arc::new(JsonDocumentWriter::for_base_dir(base_dir))
}

fn hold(base_dir: &std::path::Path, path: &std::path::Path) -> Box<dyn DocumentLock> {
    under_test(base_dir).exclusive(path).unwrap()
}

#[test]
fn the_exclusive_hold_blocks_a_second_holder_until_it_is_released() {
    let base = TempDir::new().unwrap();
    let path = base.path().join("config.json");
    let first = hold(base.path(), &path);
    let (tx, rx) = std::sync::mpsc::channel();
    let contender = {
        let base_dir = base.path().to_path_buf();
        let path = path.clone();
        std::thread::spawn(move || {
            let _second = hold(&base_dir, &path);
            tx.send(()).unwrap();
        })
    };
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(300))
            .is_err(),
        "the second holder is blocked while the first is held"
    );
    drop(first);
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .expect("the second holder proceeds once the first releases");
    contender.join().unwrap();
}

#[test]
fn the_hold_needs_no_existing_document_and_creates_nothing_beside_it() {
    let base = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    let path = repo.path().join(".quecto").join("config.json");
    // The document does not exist yet: the hold still serialises its
    // creation, without creating its directory (a refused write must
    // leave a clean checkout clean).
    let first = hold(base.path(), &path);
    assert!(!path.exists(), "the hold creates no document");
    assert!(
        std::fs::read_dir(repo.path()).unwrap().next().is_none(),
        "the hold creates nothing in the document's checkout"
    );
    drop(first);
    let _again = hold(base.path(), &path);
}
