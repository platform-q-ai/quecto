use super::*;
use crate::domain::session::ContextSpillStore;
use tempfile::TempDir;

fn entry(id: &str) -> SpillEntry {
    SpillEntry {
        id: id.into(),
        tool: "bash".into(),
        input_preview: "echo hi".into(),
        tokens: 12,
        content: "hi\n".into(),
    }
}

#[tokio::test]
async fn append_updates_warmed_index_and_clear_invalidates_disk_and_cache() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(tmp.path().to_path_buf());
    let session = "s/with spaces";

    assert!(store.list_entries(session).await.unwrap().is_empty());
    store.append(session, &entry("one")).await.unwrap();
    store.append(session, &entry("two")).await.unwrap();

    let listed = store.list_entries(session).await.unwrap();
    assert_eq!(
        listed.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        vec!["one", "two"]
    );
    assert!(store.has_entries(session).await.unwrap());

    store.clear(session).await.unwrap();
    assert!(!store.has_entries(session).await.unwrap());
    assert!(store.list_entries(session).await.unwrap().is_empty());
    assert!(store.recall(session, "one").await.unwrap().is_none());

    store.clear(session).await.unwrap();
}

#[tokio::test]
async fn spill_index_from_record_and_corrupt_jsonl_is_skipped() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(tmp.path().to_path_buf());
    let rec = SpillRecord::from(&entry("idx"));
    let idx = SpillIndex::from(&rec);
    assert_eq!(idx.id, "idx");
    assert_eq!(idx.tool, "bash");
    assert_eq!(idx.tokens, 12);

    let path = store.spill_path("session");
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    let good = serde_json::to_string(&rec).unwrap();
    tokio::fs::write(&path, format!("not-json\n{}\n", good))
        .await
        .unwrap();

    let listed = store.list_entries("session").await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "idx");
    assert_eq!(
        store
            .recall("session", "idx")
            .await
            .unwrap()
            .unwrap()
            .content,
        "hi\n"
    );
}
