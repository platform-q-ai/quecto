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

#[tokio::test]
async fn spill_store_error_mapping_closures_surface_directory_and_write_failures() {
    let tmp = TempDir::new().unwrap();

    // append: create_dir_all map_err when an ancestor is a regular file.
    let file_base = tmp.path().join("file-base");
    tokio::fs::write(&file_base, b"not a dir").await.unwrap();
    let store = FileContextSpillStore::new(file_base);
    let err = store.append("s", &entry("one")).await.unwrap_err();
    assert!(
        err.to_string().contains("failed to create spill directory"),
        "{err}"
    );

    // append: OpenOptions::open map_err when spill.jsonl is a directory.
    let store = FileContextSpillStore::new(tmp.path().to_path_buf());
    let path = store.spill_path("dir-spill");
    tokio::fs::create_dir_all(&path).await.unwrap();
    let err = store.append("dir-spill", &entry("two")).await.unwrap_err();
    assert!(
        err.to_string().contains("failed to open spill file"),
        "{err}"
    );

    // read_spill_content via has_entries: read_to_string map_err on a directory.
    let err = store.has_entries("dir-spill").await.unwrap_err();
    assert!(
        err.to_string().contains("failed to read spill file"),
        "{err}"
    );

    // clear: metadata map_err when an ancestor is a regular file.
    let blocked_base = tmp.path().join("blocked-base");
    tokio::fs::create_dir(&blocked_base).await.unwrap();
    tokio::fs::write(blocked_base.join("sessions"), b"not a dir")
        .await
        .unwrap();
    let blocked_store = FileContextSpillStore::new(blocked_base);
    let err = blocked_store.clear("child").await.unwrap_err();
    assert!(
        err.to_string().contains("failed to stat spill file"),
        "{err}"
    );
}

#[tokio::test]
async fn w5_context_spill_cache_recall_and_clear_error_paths() {
    let tmp = TempDir::new().unwrap();
    let store = FileContextSpillStore::new(tmp.path().to_path_buf());

    // Populate cache with an empty list; append updates already-warmed cache.
    assert!(store.list_entries("warm").await.unwrap().is_empty());
    store.append("warm", &entry("cached")).await.unwrap();
    let listed = store.list_entries("warm").await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "cached");

    // Recall misses via cache fast-path, substring false positive, corrupt line, and empty line.
    let path = store.spill_path("scan");
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    let rec = serde_json::to_string(&SpillRecord::from(&SpillEntry {
        id: "real".into(),
        tool: "bash".into(),
        input_preview: "preview".into(),
        tokens: 1,
        content: "mentions target but id differs".into(),
    }))
    .unwrap();
    tokio::fs::write(&path, format!("\n{{bad json target\n{rec}\n"))
        .await
        .unwrap();
    assert!(store.recall("scan", "target").await.unwrap().is_none());
    assert_eq!(
        store.recall("scan", "real").await.unwrap().unwrap().content,
        "mentions target but id differs"
    );

    assert!(store.list_entries("cached-miss").await.unwrap().is_empty());
    assert!(store.recall("cached-miss", "nope").await.unwrap().is_none());

    // clear: temp write failure when parent is a directory with colliding tmp path
    // is not deterministic because UUID is random; cover rename failure by making
    // the target path a directory after metadata succeeds and the temp write works.
    let clear_path = store.spill_path("dir-target");
    tokio::fs::create_dir_all(&clear_path).await.unwrap();
    let err = store.clear("dir-target").await.unwrap_err();
    assert!(
        err.to_string().contains("failed to write temp clear file")
            || err
                .to_string()
                .contains("failed to atomically clear spill file"),
        "{err}"
    );

    FileContextSpillStore::scrub_session_spill_sync(tmp.path(), "warm");
    assert!(!store.spill_path("warm").exists());
}
