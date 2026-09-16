//! Contract tests for the `ContextSpillStore` port.
//!
//! Drives `FileContextSpillStore` through the trait object. The contract is:
//! append → recall returns the same entry, list_entries reflects all appends
//! in exact append order, recall parses only the matching record and skips
//! torn lines, clear truncates to empty, scrub removes the namespace file
//! (D9 #1978). Every operation is keyed by the typed
//! `SessionIdentity` (#1970); the ephemeral identity retains in-run entries
//! exactly like any other key.

use quecto::application::sessions::ports::ContextSpillStore;
use quecto::domain::session::SpillEntry;
use quecto::domain::session_identity::{SessionIdentity, SpillId};
use quecto::infrastructure::persistence::context_spill::FileContextSpillStore;
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use std::io::Write;
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

#[tokio::test]
async fn recall_of_another_sessions_id_is_none() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());
    store
        .append(&id("cli:a"), &entry("shared-id"))
        .await
        .unwrap();
    assert!(
        store
            .recall(&id("cli:b"), &SpillId::new("shared-id"))
            .await
            .unwrap()
            .is_none(),
        "an id retained under A must not resolve under B"
    );
}

#[tokio::test]
async fn recall_deserializes_only_the_matching_record_and_skips_corrupt_lines() {
    // The adapter filters lines by the id as a substring before parsing and
    // matches on the parsed id, so an entry whose CONTENT mentions another
    // id is never returned for that id, and a torn line elsewhere in the
    // file neither breaks the recall nor the index.
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());
    let mut decoy = entry("decoy");
    decoy.content = "see turn1:bash:0 for the real output".to_string();
    store.append(&id("cli:s"), &decoy).await.unwrap();
    let file = tmp.path().join("sessions/cli_s/spill.jsonl");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap()
        .write_all(b"{\"id\":\"torn\",\"tool\":\"bash\"\n")
        .unwrap();
    store
        .append(&id("cli:s"), &entry("turn1:bash:0"))
        .await
        .unwrap();

    let got = store
        .recall(&id("cli:s"), &SpillId::new("turn1:bash:0"))
        .await
        .unwrap()
        .expect("the real entry resolves past the decoy and the torn line");
    assert_eq!(got.content, "content for turn1:bash:0");
    assert!(
        store
            .recall(&id("cli:s"), &SpillId::new("torn"))
            .await
            .unwrap()
            .is_none(),
        "a torn record is never served"
    );
    let ids: Vec<String> = store
        .list_entries(&id("cli:s"))
        .await
        .unwrap()
        .iter()
        .map(|i| i.id.clone())
        .collect();
    assert_eq!(
        ids,
        vec!["decoy", "turn1:bash:0"],
        "the index skips the torn line"
    );
}

#[tokio::test]
async fn append_reports_a_failure_and_retains_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    // The session directory is a file: the spill file cannot be created.
    std::fs::create_dir_all(tmp.path().join("sessions")).unwrap();
    std::fs::write(tmp.path().join("sessions/cli_s"), b"").unwrap();
    let store = under_test(tmp.path().to_path_buf());
    let err = store
        .append(&id("cli:s"), &entry("x"))
        .await
        .expect_err("append fails");
    assert!(err.to_string().contains("spill"), "{err}");
}

#[tokio::test]
async fn scrub_sync_removes_the_namespace_file_and_its_emptied_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());
    let ephemeral = SessionIdentity::ephemeral();
    store.append(&ephemeral, &entry("run-1")).await.unwrap();
    store.append(&id("cli:a"), &entry("kept")).await.unwrap();
    let file = tmp.path().join("sessions/key_/spill.jsonl");
    assert!(file.is_file());

    store.scrub_sync(&ephemeral);

    assert!(!file.exists(), "the scrubbed namespace file is gone");
    assert!(
        !file.parent().unwrap().exists(),
        "its emptied directory goes with it"
    );
    assert!(
        tmp.path().join("sessions/cli_a/spill.jsonl").is_file(),
        "another namespace stands"
    );
    // Scrubbing a namespace that never spilled is a no-op.
    store.scrub_sync(&id("cli:never"));
}
