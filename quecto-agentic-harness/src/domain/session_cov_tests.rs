use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct InMemorySpillStore {
    entries: Mutex<Vec<SpillEntry>>,
}

impl ContextSpillStore for InMemorySpillStore {
    fn append(
        &self,
        _session_key: &str,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        let entry = entry.clone();
        Box::pin(async move {
            self.entries.lock().unwrap().push(entry);
            Ok(())
        })
    }

    fn recall(
        &self,
        _session_key: &str,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>> {
        let id = id.to_string();
        Box::pin(async move {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .iter()
                .find(|entry| entry.id == id)
                .cloned())
        })
    }

    fn list_entries(&self, _session_key: &str) -> SpillIndexList<'_> {
        Box::pin(async move {
            Ok(Arc::new(
                self.entries
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|entry| SpillIndex {
                        id: entry.id.clone(),
                        tool: entry.tool.clone(),
                        input_preview: entry.input_preview.clone(),
                        tokens: entry.tokens,
                    })
                    .collect(),
            ))
        })
    }

    fn clear(
        &self,
        _session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        Box::pin(async move {
            self.entries.lock().unwrap().clear();
            Ok(())
        })
    }
}

#[tokio::test]
async fn context_spill_store_default_has_entries_reflects_list_entries() {
    let store = InMemorySpillStore::default();
    assert!(!store.has_entries("s").await.expect("empty list succeeds"));

    store
        .append(
            "s",
            &SpillEntry {
                id: "id1".to_string(),
                tool: "bash".to_string(),
                input_preview: "echo".to_string(),
                tokens: 3,
                content: "out".to_string(),
            },
        )
        .await
        .expect("append succeeds");

    assert!(
        store
            .has_entries("s")
            .await
            .expect("non-empty list succeeds")
    );
}

#[tokio::test]
async fn in_memory_spill_store_trait_surface_recalls_and_clears() {
    let store = InMemorySpillStore::default();
    let entry = SpillEntry {
        id: "id-clear".to_string(),
        tool: "grep".to_string(),
        input_preview: "needle".to_string(),
        tokens: 7,
        content: "haystack".to_string(),
    };

    store.append("s", &entry).await.expect("append");
    assert_eq!(
        store
            .recall("s", "id-clear")
            .await
            .expect("recall")
            .unwrap()
            .content,
        "haystack"
    );
    store.clear("s").await.expect("clear");
    assert!(
        store
            .recall("s", "id-clear")
            .await
            .expect("recall missing")
            .is_none()
    );
}
