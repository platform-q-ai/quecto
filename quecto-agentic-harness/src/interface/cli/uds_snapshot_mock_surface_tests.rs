use super::snapshot_tests::BlockingSpillStore;
use crate::domain::session::{ContextSpillStore, SpillEntry};

fn store_with(entry: SpillEntry) -> BlockingSpillStore {
    BlockingSpillStore {
        entry,
        started: std::sync::Mutex::new(None),
        release: std::sync::Mutex::new(None),
    }
}

#[tokio::test]
async fn blocking_spill_store_covers_append_list_clear_and_recall() {
    let store = store_with(SpillEntry {
        id: "spill".into(),
        tool: "message".into(),
        input_preview: String::new(),
        tokens: 3,
        content: "full".into(),
    });

    store.append("cli:snap", &store.entry).await.unwrap();
    assert!(store.list_entries("cli:snap").await.unwrap().is_empty());
    assert_eq!(
        store
            .recall("cli:snap", "spill")
            .await
            .unwrap()
            .unwrap()
            .content,
        "full"
    );
    store.clear("cli:snap").await.unwrap();
}

#[tokio::test]
async fn blocking_spill_store_default_has_entries_is_false() {
    let store = store_with(SpillEntry {
        id: "spill".into(),
        tool: "message".into(),
        input_preview: String::new(),
        tokens: 3,
        content: "full".into(),
    });

    assert!(!store.has_entries("cli:snap").await.unwrap());
}
