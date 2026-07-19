use super::tests_1093::MemSpillStore;
use crate::domain::session::{ContextSpillStore, SpillEntry};

fn entry(id: &str, content: &str) -> SpillEntry {
    SpillEntry {
        id: id.into(),
        tool: "message".into(),
        input_preview: String::new(),
        tokens: 1,
        content: content.into(),
    }
}

#[tokio::test]
async fn mem_spill_store_append_recall_list_and_clear_surface() {
    let store = MemSpillStore::default();
    store.append("cli:surface", &entry("a", "A")).await.unwrap();

    let recalled = store.recall("cli:surface", "a").await.unwrap().unwrap();
    assert_eq!(recalled.content, "A");
    assert!(store.list_entries("cli:surface").await.unwrap().is_empty());
    assert_eq!(store.recall_count(), 1);
    assert_eq!(store.recalled(), vec![("cli:surface".into(), "a".into())]);

    store.clear("cli:surface").await.unwrap();
    assert!(store.recall("cli:surface", "a").await.unwrap().is_none());
}

#[tokio::test]
async fn mem_spill_store_recall_error_surface() {
    let store = MemSpillStore::with_recall_error();
    let err = store.recall("cli:surface", "missing").await.unwrap_err();
    assert!(err.to_string().contains("boom"));
}
