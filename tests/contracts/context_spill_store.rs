//! Contract tests for the `ContextSpillStore` port.
//!
//! Drives `FileContextSpillStore` through the trait object. The contract is:
//! append → recall returns the same entry, list_entries reflects all appends,
//! clear truncates to empty.

use quecto::domain::session::{ContextSpillStore, SpillEntry};
use quecto::infrastructure::persistence::context_spill::FileContextSpillStore;
use std::sync::Arc;

fn under_test(base_dir: std::path::PathBuf) -> Arc<dyn ContextSpillStore> {
    Arc::new(FileContextSpillStore::new(base_dir))
}

fn entry(id: &str) -> SpillEntry {
    SpillEntry {
        id: id.to_string(),
        tool: "bash".to_string(),
        input_preview: "ls".to_string(),
        tokens: 10,
        content: format!("content for {id}"),
    }
}

#[tokio::test]
async fn recall_returns_none_for_unknown_id() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());
    let out = store.recall("cli:s", "missing").await.unwrap();
    assert!(out.is_none());
}

#[tokio::test]
async fn append_then_recall_returns_same_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());
    let e = entry("sp-1");
    store.append("cli:s", &e).await.unwrap();

    let got = store
        .recall("cli:s", "sp-1")
        .await
        .unwrap()
        .expect("recall must return Some after append");
    assert_eq!(got.id, e.id);
    assert_eq!(got.content, e.content);
    assert_eq!(got.tokens, e.tokens);
}

#[tokio::test]
async fn list_entries_covers_every_append_in_order() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());
    store.append("cli:s", &entry("a")).await.unwrap();
    store.append("cli:s", &entry("b")).await.unwrap();
    store.append("cli:s", &entry("c")).await.unwrap();

    let listed = store.list_entries("cli:s").await.unwrap();
    let ids: Vec<&str> = listed.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["a", "b", "c"],
        "list_entries must preserve append order"
    );
}

#[tokio::test]
async fn clear_empties_the_session_spill() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());
    store.append("cli:s", &entry("x")).await.unwrap();

    store.clear("cli:s").await.unwrap();
    assert!(
        store.list_entries("cli:s").await.unwrap().is_empty(),
        "clear must truncate the session spill to empty"
    );
    assert!(
        store.recall("cli:s", "x").await.unwrap().is_none(),
        "recall must not find cleared entries"
    );
}

#[tokio::test]
async fn session_keys_are_isolated() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());
    store.append("cli:a", &entry("only-a")).await.unwrap();

    let other = store.list_entries("cli:b").await.unwrap();
    assert!(
        other.is_empty(),
        "a spill appended to session A must not be visible in session B"
    );
}
