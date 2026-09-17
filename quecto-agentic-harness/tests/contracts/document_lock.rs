//! Contract for the `DocumentLock` port (#2024): the hold a writer hands
//! out serialises read → patch → write cycles — a second holder of the
//! same document waits until the first is dropped, whichever thread or
//! process holds it.
use std::sync::Arc;
use tempfile::TempDir;

use quecto::application::configuration::ports::{ConfigDocumentWriter, DocumentLock};
use quecto::infrastructure::config::writer::JsonDocumentWriter;

fn under_test() -> Arc<dyn ConfigDocumentWriter> {
    Arc::new(JsonDocumentWriter)
}

fn hold(path: &std::path::Path) -> Box<dyn DocumentLock> {
    under_test().exclusive(path).unwrap()
}

#[test]
fn the_exclusive_hold_blocks_a_second_holder_until_it_is_released() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    let first = hold(&path);
    let (tx, rx) = std::sync::mpsc::channel();
    let contender = {
        let path = path.clone();
        std::thread::spawn(move || {
            let _second = hold(&path);
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
fn the_hold_is_reentrant_across_names_of_one_file_and_needs_no_existing_document() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested").join("config.json");
    // The document does not exist yet: the hold still serialises its creation.
    let first = hold(&path);
    assert!(path.with_extension("json.lock").exists());
    assert!(!path.exists(), "the hold creates no document");
    drop(first);
    let _again = hold(&path);
}
