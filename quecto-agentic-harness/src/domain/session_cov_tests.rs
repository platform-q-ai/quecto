use super::*;
use crate::domain::message::Role;
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

#[derive(Default)]
struct DefaultDeltaStore {
    saved: Mutex<Vec<Session>>,
}

impl SessionStore for DefaultDeltaStore {
    fn load(
        &self,
        _key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Session>, DomainError>> + Send + '_>> {
        Box::pin(async { Ok(None) })
    }

    fn save(
        &self,
        session: &Session,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        let session = session.clone();
        Box::pin(async move {
            self.saved.lock().unwrap().push(session);
            Ok(())
        })
    }

    fn exists(
        &self,
        _key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DomainError>> + Send + '_>> {
        Box::pin(async { Ok(false) })
    }

    fn list(
        &self,
        _key_prefix: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SessionSummary>, DomainError>> + Send + '_>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[tokio::test]
async fn default_session_store_delta_methods_delegate_to_save() {
    let store = DefaultDeltaStore::default();
    let messages = vec![Message::user("hello"), Message::assistant("world", vec![])];
    let workflow = crate::domain::workflow::WorkflowRunPersisted {
        template_id: Some("review".into()),
        done: vec![true, false],
        active_issue: Some((1247, "delta fallback".into())),
    };

    store
        .save_delta("chat-a", &messages, 999, Some(workflow.clone()))
        .await
        .expect("default save_delta succeeds");
    store
        .save_clean_delta("chat-b", &messages, 1, Some(workflow.clone()))
        .await
        .expect("default save_clean_delta succeeds");

    let saved = store.saved.lock().unwrap();
    assert_eq!(saved.len(), 2);
    assert_eq!(saved[0].key, "chat-a");
    assert_eq!(saved[1].key, "chat-b");
    assert_eq!(saved[0].messages.len(), 2);
    assert_eq!(saved[1].messages.len(), 2);
    assert_eq!(saved[0].messages[0].role, Role::User);
    assert_eq!(saved[0].messages[0].content, "hello");
    assert_eq!(saved[0].messages[1].role, Role::Assistant);
    assert_eq!(saved[0].messages[1].content, "world");
    assert_eq!(saved[1].messages[0].role, Role::User);
    assert_eq!(saved[1].messages[0].content, "hello");
    assert_eq!(saved[1].messages[1].role, Role::Assistant);
    assert_eq!(saved[1].messages[1].content, "world");
    assert_eq!(saved[0].workflow_run, Some(workflow.clone()));
    assert_eq!(saved[1].workflow_run, Some(workflow));
}
