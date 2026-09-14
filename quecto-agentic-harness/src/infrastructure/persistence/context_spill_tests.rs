use super::*;
use crate::domain::session_identity::{SessionIdentity, SpillId};
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use tempfile::TempDir;

fn id(k: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(k)
}

fn test_entry() -> SpillEntry {
    SpillEntry {
        id: "turn1:bash:0".to_string(),
        tool: "bash".to_string(),
        input_preview: "echo hello".to_string(),
        tokens: 100,
        content: "hello\n".to_string(),
    }
}

#[test]
fn debug_names_base_dir_without_dumping_cache_contents() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(tmp.path().join("spill-base")));
    let debug = format!("{store:?}");
    assert!(
        debug.contains("spill-base"),
        "debug should identify the store base dir: {debug}"
    );
    assert!(
        !debug.contains("index_cache"),
        "debug must not dump cached spill ids/content: {debug}"
    );
}

#[tokio::test]
async fn list_entries_seeds_empty_cache_and_append_extends_it() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(tmp.path()));
    let initial = store.list_entries(&id("new-session")).await.unwrap();
    assert!(initial.is_empty());
    let mut entry = test_entry();
    entry.id = "turn9:read:0".into();
    store.append(&id("new-session"), &entry).await.unwrap();
    let entries = store.list_entries(&id("new-session")).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "turn9:read:0");
    assert_eq!(entries[0].tool, "bash");
    assert_eq!(entries[0].tokens, 100);
}

#[tokio::test]
async fn recall_cache_miss_short_circuits_even_if_disk_contains_id() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(tmp.path()));
    store.append(&id("session"), &test_entry()).await.unwrap();
    assert_eq!(store.list_entries(&id("session")).await.unwrap().len(), 1);
    let mut hidden = test_entry();
    hidden.id = "turn-hidden:bash:0".into();
    let path = store.spill_path(&id("session"));
    let mut line = serde_json::to_string(&SpillRecord::from(&hidden)).unwrap();
    line.push('\n');
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .await
        .unwrap();
    file.write_all(line.as_bytes()).await.unwrap();
    file.flush().await.unwrap();
    let recalled = store
        .recall(&id("session"), &SpillId::new("turn-hidden:bash:0"))
        .await
        .unwrap();
    assert!(
        recalled.is_none(),
        "populated index cache should avoid scanning IDs not present in it"
    );
}

#[tokio::test]
async fn test_append_and_recall() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(tmp.path()));
    let entry = test_entry();

    store.append(&id("test-session"), &entry).await.unwrap();

    let recalled = store
        .recall(&id("test-session"), &SpillId::new("turn1:bash:0"))
        .await
        .unwrap();
    assert!(recalled.is_some());
    let recalled = recalled.unwrap();
    assert_eq!(recalled.id, "turn1:bash:0");
    assert_eq!(recalled.content, "hello\n");
}

#[tokio::test]
async fn test_recall_not_found() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(tmp.path()));

    let recalled = store
        .recall(&id("test-session"), &SpillId::new("nonexistent"))
        .await
        .unwrap();
    assert!(recalled.is_none());
}

#[tokio::test]
async fn test_list_entries() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(tmp.path()));

    let entry1 = SpillEntry {
        id: "turn1:bash:0".to_string(),
        tool: "bash".to_string(),
        input_preview: "echo hello".to_string(),
        tokens: 100,
        content: "hello\n".to_string(),
    };
    let entry2 = SpillEntry {
        id: "turn2:bash:0".to_string(),
        tool: "bash".to_string(),
        input_preview: "ls -la".to_string(),
        tokens: 200,
        content: "total 0\n".to_string(),
    };

    store.append(&id("test-session"), &entry1).await.unwrap();
    store.append(&id("test-session"), &entry2).await.unwrap();

    let entries = store.list_entries(&id("test-session")).await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].id, "turn1:bash:0");
    assert_eq!(entries[1].id, "turn2:bash:0");
    // Index entries should not have content
}

#[tokio::test]
async fn test_sanitize_session_key() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(tmp.path()));
    let entry = test_entry();

    // Session keys with special characters should work (colon is sanitized to _)
    store.append(&id("telegram:12345"), &entry).await.unwrap();

    let recalled = store
        .recall(&id("telegram:12345"), &SpillId::new("turn1:bash:0"))
        .await
        .unwrap();
    assert!(recalled.is_some());
}

#[tokio::test]
async fn test_clear_truncates_spill_file() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(tmp.path()));
    let entry = test_entry();

    store.append(&id("test-session"), &entry).await.unwrap();

    // Verify entry is present before clearing
    let entries = store.list_entries(&id("test-session")).await.unwrap();
    assert_eq!(entries.len(), 1);

    // Clear the spill
    store.clear(&id("test-session")).await.unwrap();

    // Verify the file is now empty
    let entries_after = store.list_entries(&id("test-session")).await.unwrap();
    assert!(entries_after.is_empty());
}

#[tokio::test]
async fn test_has_entries_ignores_a_torn_write_with_no_parseable_entries() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(dir.path()));
    let path = store.spill_path(&id("session"));
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&path, br#"{"id":"turn1:bash:0""#)
        .await
        .unwrap();

    assert!(store.list_entries(&id("session")).await.unwrap().is_empty());
    assert!(
        !store.has_entries(&id("session")).await.unwrap(),
        "presence must agree with the parse-based index"
    );
}

#[tokio::test]
async fn test_has_entries_tracks_append_and_clear_without_loading_index() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(dir.path()));
    assert!(!store.has_entries(&id("session")).await.unwrap());
    store
        .append(
            &id("session"),
            &SpillEntry {
                id: "turn1:bash:0".into(),
                tool: "bash".into(),
                input_preview: "echo hi".into(),
                tokens: 2,
                content: "hi".into(),
            },
        )
        .await
        .unwrap();
    assert!(store.has_entries(&id("session")).await.unwrap());
    store.clear(&id("session")).await.unwrap();
    assert!(!store.has_entries(&id("session")).await.unwrap());
}

#[tokio::test]
async fn test_clear_nonexistent_file_is_noop() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(tmp.path()));

    // Clearing a non-existent session's spill file should not error
    let result = store.clear(&id("ghost-session")).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_list_entries_uses_cache_after_append() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(tmp.path()));

    let entry = test_entry();
    store.append(&id("cached-session"), &entry).await.unwrap();

    // Seed the cache via list_entries (simulates agent loop startup)
    let initial = store.list_entries(&id("cached-session")).await.unwrap();
    assert_eq!(initial.len(), 1);

    // Append a second entry (updates cache)
    let entry2 = SpillEntry {
        id: "turn2:bash:0".to_string(),
        tool: "bash".to_string(),
        input_preview: "ls".to_string(),
        tokens: 50,
        content: "file.txt\n".to_string(),
    };
    store.append(&id("cached-session"), &entry2).await.unwrap();

    // Delete the spill file behind the store's back
    let spill_path = store.spill_path(&id("cached-session"));
    tokio::fs::remove_file(&spill_path).await.unwrap();

    // list_entries should still return both entries from cache
    let entries = store.list_entries(&id("cached-session")).await.unwrap();
    assert_eq!(
        entries.len(),
        2,
        "list_entries should return cached entries even after disk file is deleted"
    );
    assert_eq!(entries[0].id, "turn1:bash:0");
    assert_eq!(entries[1].id, "turn2:bash:0");
}

#[tokio::test]
async fn test_clear_invalidates_cache() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(tmp.path()));

    let entry = test_entry();
    store.append(&id("clear-test"), &entry).await.unwrap();

    // Verify cached
    let entries = store.list_entries(&id("clear-test")).await.unwrap();
    assert_eq!(entries.len(), 1);

    // Clear should invalidate cache
    store.clear(&id("clear-test")).await.unwrap();

    let entries = store.list_entries(&id("clear-test")).await.unwrap();
    assert!(entries.is_empty(), "cache should be cleared after clear()");
}

#[tokio::test]
async fn test_recall_finds_entry_among_many() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(tmp.path()));

    // Append 10 entries with distinct content
    for i in 0..10 {
        let entry = SpillEntry {
            id: format!("turn{}:bash:0", i + 1),
            tool: "bash".to_string(),
            input_preview: format!("cmd-{}", i + 1),
            tokens: 100,
            content: format!("output-{}-{}", i + 1, "x".repeat(1000)),
        };
        store.append(&id("recall-test"), &entry).await.unwrap();
    }

    // Recall the 5th entry
    let recalled = store
        .recall(&id("recall-test"), &SpillId::new("turn5:bash:0"))
        .await
        .unwrap();
    assert!(recalled.is_some(), "should find turn5:bash:0");
    let entry = recalled.unwrap();
    assert_eq!(entry.id, "turn5:bash:0");
    assert!(
        entry.content.starts_with("output-5-"),
        "content should match the 5th entry"
    );

    // Recall nonexistent
    let missing = store
        .recall(&id("recall-test"), &SpillId::new("turn99:bash:0"))
        .await
        .unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn scrub_session_spill_sync_removes_ephemeral_spill_file_and_dir() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(tmp.path()));
    // Ephemeral runs spill under the sanitized empty key.
    store.append(&id(""), &test_entry()).await.unwrap();
    let path = store.spill_path(&id(""));
    assert!(path.exists(), "positive control: the spill file must exist");

    FileContextSpillStore::scrub_session_spill_sync(&FlatSessionLayout::new(tmp.path()), &id(""));

    assert!(
        !path.exists(),
        "ephemeral spill content must not outlive the run (PR #1048 security review)"
    );
    assert!(
        !path.parent().unwrap().exists(),
        "the emptied ephemeral session dir must be removed too"
    );
}

#[test]
fn scrub_session_spill_sync_is_a_noop_when_nothing_was_spilled() {
    let tmp = TempDir::new().unwrap();
    // Must not panic or create anything when no spill file exists.
    FileContextSpillStore::scrub_session_spill_sync(&FlatSessionLayout::new(tmp.path()), &id(""));
    assert!(!tmp.path().join("sessions").exists());
}

#[tokio::test]
async fn test_recall_handles_id_substring_in_content() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(FlatSessionLayout::new(tmp.path()));

    // Entry whose content contains another entry's ID as a substring
    let entry1 = SpillEntry {
        id: "turn1:bash:0".to_string(),
        tool: "bash".to_string(),
        input_preview: "echo".to_string(),
        tokens: 50,
        content: "Use recall(\"turn2:bash:0\") to see the other output".to_string(),
    };
    let entry2 = SpillEntry {
        id: "turn2:bash:0".to_string(),
        tool: "bash".to_string(),
        input_preview: "ls".to_string(),
        tokens: 50,
        content: "actual output".to_string(),
    };
    store.append(&id("substr-test"), &entry1).await.unwrap();
    store.append(&id("substr-test"), &entry2).await.unwrap();

    // Recalling turn2 should return entry2, not entry1 (even though
    // entry1's content contains "turn2:bash:0" as a substring)
    let recalled = store
        .recall(&id("substr-test"), &SpillId::new("turn2:bash:0"))
        .await
        .unwrap();
    assert!(recalled.is_some());
    let entry = recalled.unwrap();
    assert_eq!(entry.id, "turn2:bash:0");
    assert_eq!(entry.content, "actual output");
}
