//! Contract tests for the `ContextSpillStore` port.
//!
//! Drives `FileContextSpillStore` through the trait object. The contract is:
//! append → recall returns the same entry, list_entries reflects all appends,
//! clear truncates to empty. Every operation is keyed by the typed
//! `SessionIdentity` (#1970); the ephemeral identity retains in-run entries
//! exactly like any other key.

use quecto::application::sessions::ports::ContextSpillStore;
use quecto::domain::session::SpillEntry;
use quecto::domain::session_identity::{SessionIdentity, SpillId};
use quecto::infrastructure::persistence::context_spill::FileContextSpillStore;
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use std::sync::Arc;

fn under_test(base_dir: std::path::PathBuf) -> Arc<dyn ContextSpillStore> {
    Arc::new(FileContextSpillStore::new(FlatSessionLayout::new(base_dir)))
}

fn id(key: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(key)
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
    let out = store
        .recall(&id("cli:s"), &SpillId::new("missing"))
        .await
        .unwrap();
    assert!(out.is_none());
}

#[tokio::test]
async fn append_then_recall_returns_same_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());
    let e = entry("sp-1");
    store.append(&id("cli:s"), &e).await.unwrap();

    let got = store
        .recall(&id("cli:s"), &SpillId::new("sp-1"))
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
    store.append(&id("cli:s"), &entry("a")).await.unwrap();
    store.append(&id("cli:s"), &entry("b")).await.unwrap();
    store.append(&id("cli:s"), &entry("c")).await.unwrap();

    let listed = store.list_entries(&id("cli:s")).await.unwrap();
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
    store.append(&id("cli:s"), &entry("x")).await.unwrap();

    store.clear(&id("cli:s")).await.unwrap();
    assert!(
        store.list_entries(&id("cli:s")).await.unwrap().is_empty(),
        "clear must truncate the session spill to empty"
    );
    assert!(
        store
            .recall(&id("cli:s"), &SpillId::new("x"))
            .await
            .unwrap()
            .is_none(),
        "recall must not find cleared entries"
    );
}

#[tokio::test]
async fn session_keys_are_isolated() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());
    store.append(&id("cli:a"), &entry("only-a")).await.unwrap();

    let other = store.list_entries(&id("cli:b")).await.unwrap();
    assert!(
        other.is_empty(),
        "a spill appended to session A must not be visible in session B"
    );
}

#[tokio::test]
async fn ephemeral_identity_retains_in_run_entries_under_the_sanitized_empty_key() {
    // `--no-session` runs spill for their own recall() stubs; the ephemeral
    // identity is a full retention namespace of its own, isolated from every
    // named session, and it lives where the empty key always did.
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());
    let ephemeral = SessionIdentity::ephemeral();
    store.append(&ephemeral, &entry("run-1")).await.unwrap();
    assert!(store.has_entries(&ephemeral).await.unwrap());
    assert!(!store.has_entries(&id("cli:a")).await.unwrap());
    assert_eq!(
        store
            .recall(&ephemeral, &SpillId::new("run-1"))
            .await
            .unwrap()
            .map(|e| e.content),
        Some("content for run-1".to_string())
    );
    assert!(tmp.path().join("sessions/key_/spill.jsonl").is_file());
}
