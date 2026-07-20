use super::*;
use crate::domain::session::{SpillEntry, SpillIndex};

// In-memory spill store for testing
#[derive(Debug)]
struct MemorySpillStore {
    entries: Mutex<Vec<SpillEntry>>,
}

impl MemorySpillStore {
    fn new() -> Self {
        Self {
            entries: Mutex::new(vec![]),
        }
    }

    fn add(&self, entry: SpillEntry) {
        self.entries.lock().unwrap().push(entry);
    }
}

impl ContextSpillStore for MemorySpillStore {
    fn append(
        &self,
        _session_key: &str,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.entries.lock().unwrap().push(entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        _session_key: &str,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>> {
        let id = id.to_string();
        let result = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.id == id)
            .cloned();
        Box::pin(async move { Ok(result) })
    }

    fn list_entries(
        &self,
        _session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<Vec<SpillIndex>>, DomainError>> + Send + '_>> {
        let entries: Vec<SpillIndex> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .map(|e| SpillIndex {
                id: e.id.clone(),
                tool: e.tool.clone(),
                input_preview: e.input_preview.clone(),
                tokens: e.tokens,
            })
            .collect();
        Box::pin(async move { Ok(Arc::new(entries)) })
    }

    fn clear(
        &self,
        _session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.entries.lock().unwrap().clear();
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug)]
struct KeyedMemorySpillStore {
    entries: Mutex<HashMap<String, Vec<SpillEntry>>>,
}

impl KeyedMemorySpillStore {
    fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn add(&self, session_key: &str, entry: SpillEntry) {
        self.entries
            .lock()
            .unwrap()
            .entry(session_key.to_string())
            .or_default()
            .push(entry);
    }
}

impl ContextSpillStore for KeyedMemorySpillStore {
    fn append(
        &self,
        session_key: &str,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.add(session_key, entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        session_key: &str,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>> {
        let result = self
            .entries
            .lock()
            .unwrap()
            .get(session_key)
            .and_then(|entries| entries.iter().find(|e| e.id == id).cloned());
        Box::pin(async move { Ok(result) })
    }

    fn list_entries(
        &self,
        session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<Vec<SpillIndex>>, DomainError>> + Send + '_>> {
        let entries: Vec<SpillIndex> = self
            .entries
            .lock()
            .unwrap()
            .get(session_key)
            .into_iter()
            .flatten()
            .map(|e| SpillIndex {
                id: e.id.clone(),
                tool: e.tool.clone(),
                input_preview: e.input_preview.clone(),
                tokens: e.tokens,
            })
            .collect();
        Box::pin(async move { Ok(Arc::new(entries)) })
    }

    fn clear(
        &self,
        session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.entries.lock().unwrap().remove(session_key);
        Box::pin(async { Ok(()) })
    }
}

fn test_store_with_entry() -> Arc<MemorySpillStore> {
    let store = Arc::new(MemorySpillStore::new());
    store.add(SpillEntry {
        id: "turn5:bash:0".to_string(),
        tool: "bash".to_string(),
        input_preview: "echo hello".to_string(),
        tokens: 100,
        content: "hello world output".to_string(),
    });
    store
}

#[tokio::test]
async fn test_recall_by_id() {
    // Restored to its original single purpose: recalling the seeded entry.
    // Store lifecycle (has_entries/list_entries/clear) is covered separately
    // by test_spill_store_lifecycle below.
    let store = test_store_with_entry();
    let tool = RecallTool::new(store, "test-session".to_string());
    let result = tool.execute(r#"{"id":"turn5:bash:0"}"#).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content, "hello world output");
}

#[tokio::test]
async fn test_spill_store_lifecycle() {
    let store = test_store_with_entry();
    assert!(store.has_entries("test-session").await.unwrap());

    let appended = SpillEntry {
        id: "turn7:grep:0".to_string(),
        tool: "grep".to_string(),
        input_preview: "pattern".to_string(),
        tokens: 7,
        content: "match".to_string(),
    };
    store.append("test-session", &appended).await.unwrap();
    let listed = store.list_entries("test-session").await.unwrap();
    assert!(listed.iter().any(|e| e.id == "turn7:grep:0"));

    store.clear("test-session").await.unwrap();
    assert!(!store.has_entries("test-session").await.unwrap());

    // Appending after a clear starts a fresh, recallable generation.
    store.append("test-session", &appended).await.unwrap();
    let tool = RecallTool::new(store, "test-session".to_string());
    let result = tool.execute(r#"{"id":"turn7:grep:0"}"#).await.unwrap();
    assert_eq!(result.content, "match");
}

#[tokio::test]
async fn test_recall_uses_updated_session_key() {
    let store = Arc::new(KeyedMemorySpillStore::new());
    store.add(
        "old-session",
        SpillEntry {
            id: "turn1:bash:0".to_string(),
            tool: "bash".to_string(),
            input_preview: "old".to_string(),
            tokens: 1,
            content: "old output".to_string(),
        },
    );
    store.add(
        "new-session",
        SpillEntry {
            id: "turn1:bash:0".to_string(),
            tool: "bash".to_string(),
            input_preview: "new".to_string(),
            tokens: 1,
            content: "new output".to_string(),
        },
    );
    assert!(store.has_entries("old-session").await.unwrap());
    assert!(store.has_entries("new-session").await.unwrap());
    let extra = SpillEntry {
        id: "turn2:bash:0".to_string(),
        tool: "bash".to_string(),
        input_preview: "extra".to_string(),
        tokens: 2,
        content: "extra output".to_string(),
    };
    store.append("new-session", &extra).await.unwrap();
    assert_eq!(store.list_entries("new-session").await.unwrap().len(), 2);
    store.clear("old-session").await.unwrap();
    assert!(!store.has_entries("old-session").await.unwrap());
    assert!(store.has_entries("new-session").await.unwrap());

    let tool = RecallTool::new(store, "old-session".to_string());

    tool.set_session_key("new-session".to_string());

    let result = tool.execute(r#"{"id":"turn1:bash:0"}"#).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content, "new output");
}

#[tokio::test]
async fn test_recall_not_found() {
    let store = test_store_with_entry();
    let tool = RecallTool::new(store, "test-session".to_string());
    let result = tool.execute(r#"{"id":"nonexistent:id:0"}"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("No spilled output found"));
}

#[tokio::test]
async fn test_recall_list() {
    let store = test_store_with_entry();
    store.add(SpillEntry {
        id: "turn6:bash:0".to_string(),
        tool: "bash".to_string(),
        input_preview: "ls -la".to_string(),
        tokens: 200,
        content: "drwxr-xr-x".to_string(),
    });
    let tool = RecallTool::new(store, "test-session".to_string());
    let result = tool.execute(r#"{"id":"list"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("2 entries"));
    assert!(result.content.contains("turn5:bash:0"));
    assert!(result.content.contains("turn6:bash:0"));
    // List should NOT contain full content
    assert!(!result.content.contains("hello world output"));
    assert!(!result.content.contains("drwxr-xr-x"));
}

#[tokio::test]
async fn test_recall_list_empty() {
    let store = Arc::new(MemorySpillStore::new());
    let tool = RecallTool::new(store, "test-session".to_string());
    let result = tool.execute(r#"{"id":"list"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("No spilled outputs"));
}

#[test]
fn test_extract_id() {
    assert_eq!(extract_id(r#"{"id":"turn5:bash:0"}"#), "turn5:bash:0");
    assert_eq!(extract_id(r#"{"id":"list"}"#), "list");
    assert_eq!(extract_id(r#"{}"#), "");
    assert_eq!(extract_id("invalid"), "");
}

#[test]
fn test_tool_definition() {
    let store = Arc::new(MemorySpillStore::new());
    let tool = RecallTool::new(store, "test".to_string());
    let def = tool.definition();
    assert_eq!(def.name, "recall");
    assert!(def.description.contains("spilled session memory"));
    assert!(def.description.contains("full session-memory index"));
    assert!(def.description.contains("recall(\"list\")"));
}
