use super::*;
use crate::application::sessions::ports::ContextSpillStore;
use crate::domain::sessions::entities::session::{SpillEntry, SpillIndex};
use crate::domain::sessions::entities::session_identity::{SessionIdentity, SpillId};

/// The recall use case over `store`, composed as the runtime composes it
/// (D9 #1978): the tool adapts the use case, never the store.
fn recall_over(
    store: Arc<dyn crate::application::sessions::ports::ContextSpillStore>,
) -> Arc<crate::application::sessions::use_cases::RecallContext> {
    crate::composition::retention::retention_handles_over(store).recall
}

fn id(k: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(k)
}

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
        _session_key: &SessionIdentity,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.entries.lock().unwrap().push(entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        _session_key: &SessionIdentity,
        id: &SpillId,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>> {
        let id = id.as_str().to_string();
        let result = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.id == id.as_str())
            .cloned();
        Box::pin(async move { Ok(result) })
    }

    fn list_entries(
        &self,
        _session_key: &SessionIdentity,
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
        _session_key: &SessionIdentity,
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
        session_key: &SessionIdentity,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.add(session_key.runtime_key(), entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        session_key: &SessionIdentity,
        id: &SpillId,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>> {
        let result = self
            .entries
            .lock()
            .unwrap()
            .get(session_key.runtime_key())
            .and_then(|entries| entries.iter().find(|e| e.id == id.as_str()).cloned());
        Box::pin(async move { Ok(result) })
    }

    fn list_entries(
        &self,
        session_key: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<Vec<SpillIndex>>, DomainError>> + Send + '_>> {
        let entries: Vec<SpillIndex> = self
            .entries
            .lock()
            .unwrap()
            .get(session_key.runtime_key())
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
        session_key: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.entries
            .lock()
            .unwrap()
            .remove(session_key.runtime_key());
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
        images: Vec::new(),
    });
    store
}

#[tokio::test]
async fn test_recall_by_id() {
    // Restored to its original single purpose: recalling the seeded entry.
    // Store lifecycle (has_entries/list_entries/clear) is covered separately
    // by test_spill_store_lifecycle below.
    let store = test_store_with_entry();
    let tool = RecallTool::new(recall_over(store), "test-session".to_string());
    let result = tool.execute(r#"{"id":"turn5:bash:0"}"#).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content, "hello world output");
}

#[tokio::test]
async fn test_spill_store_lifecycle() {
    let store = test_store_with_entry();
    assert!(store.has_entries(&id("test-session")).await.unwrap());

    let appended = SpillEntry {
        id: "turn7:grep:0".to_string(),
        tool: "grep".to_string(),
        input_preview: "pattern".to_string(),
        tokens: 7,
        content: "match".to_string(),
        images: Vec::new(),
    };
    store.append(&id("test-session"), &appended).await.unwrap();
    let listed = store.list_entries(&id("test-session")).await.unwrap();
    assert!(listed.iter().any(|e| e.id == "turn7:grep:0"));

    store.clear(&id("test-session")).await.unwrap();
    assert!(!store.has_entries(&id("test-session")).await.unwrap());

    // Appending after a clear starts a fresh, recallable generation.
    store.append(&id("test-session"), &appended).await.unwrap();
    let tool = RecallTool::new(recall_over(store), "test-session".to_string());
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
            images: Vec::new(),
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
            images: Vec::new(),
        },
    );
    assert!(store.has_entries(&id("old-session")).await.unwrap());
    assert!(store.has_entries(&id("new-session")).await.unwrap());
    let extra = SpillEntry {
        id: "turn2:bash:0".to_string(),
        tool: "bash".to_string(),
        input_preview: "extra".to_string(),
        tokens: 2,
        content: "extra output".to_string(),
        images: Vec::new(),
    };
    store.append(&id("new-session"), &extra).await.unwrap();
    assert_eq!(
        store.list_entries(&id("new-session")).await.unwrap().len(),
        2
    );
    store.clear(&id("old-session")).await.unwrap();
    assert!(!store.has_entries(&id("old-session")).await.unwrap());
    assert!(store.has_entries(&id("new-session")).await.unwrap());

    let tool = RecallTool::new(recall_over(store), "old-session".to_string());

    tool.set_session_key("new-session".to_string());

    let result = tool.execute(r#"{"id":"turn1:bash:0"}"#).await.unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content, "new output");
}

#[tokio::test]
async fn test_recall_not_found() {
    let store = test_store_with_entry();
    let tool = RecallTool::new(recall_over(store), "test-session".to_string());
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
        images: Vec::new(),
    });
    let tool = RecallTool::new(recall_over(store), "test-session".to_string());
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
    let tool = RecallTool::new(recall_over(store), "test-session".to_string());
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
    let tool = RecallTool::new(recall_over(store), "test".to_string());
    let def = tool.definition();
    assert_eq!(def.name, "recall");
    assert!(def.description.contains("spilled session memory"));
    assert!(def.description.contains("full session-memory index"));
    assert!(def.description.contains("recall(\"list\")"));
}

/// #2215: an unknown id points to recall("list") and says what ids look like.
#[tokio::test]
async fn an_unknown_id_points_to_the_list() {
    let tool = RecallTool::new(recall_over(test_store_with_entry()), "s".to_string());
    let result = tool
        .execute(r#"{"id":"qa-no-such-spill-xyz"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert_eq!(
        result.content,
        "No spilled output found for id: qa-no-such-spill-xyz. Use recall(\"list\") to see \
         the ids this session has (they look like turn12:bash:0)."
    );
}

/// #2215: bash's saved-output path is a file to open with read, not an id.
#[tokio::test]
async fn a_saved_output_path_is_sent_to_read() {
    let tool = RecallTool::new(recall_over(test_store_with_entry()), "s".to_string());
    // Paths under a temporary directory that are sure not to exist: the
    // guidance depends on the file's size when there is one (#2254 review).
    let tmp = tempfile::TempDir::new().unwrap();
    let saved = tmp.path().join("quecto-bash-output/bash-output-ydrV3Y.log");
    let notes = tmp.path().join("notes.txt");
    for path in [saved.display().to_string(), notes.display().to_string()] {
        assert!(!std::path::Path::new(&path).exists(), "{path}");
        let args = serde_json::json!({ "id": path }).to_string();
        let result = tool.execute(&args).await.unwrap();
        assert!(result.is_error);
        assert_eq!(
            result.content,
            format!(
                "No spilled output found for id: {path}. {path} looks like a file path: \
                 saved bash output is a file, so open it with read, e.g. \
                 {{\"path\":\"{path}\",\"offset\":1,\"limit\":200}}, or, if it is over \
                 read's 10.0MB limit, page through it with bash, e.g. sed -n '1,200p' \
                 '{path}'. Use recall(\"list\") to see the ids this session has (they look \
                 like turn12:bash:0)."
            )
        );
    }
}

/// #2215: only a path is taken for one; a spill id never is.
#[test]
fn only_paths_look_like_file_paths() {
    for id in ["/a", "/tmp/x.log", "x/quecto-bash-output/y.log"] {
        assert!(looks_like_file_path(id), "{id}");
    }
    for id in [
        "turn12:bash:0",
        "turn3:read:1:2",
        "list",
        "",
        "a/b",
        "quecto-bash-output",
    ] {
        assert!(!looks_like_file_path(id), "{id}");
    }
}

/// #2254 review: a path that names a file is sent to read within read's
/// cap and to bounded bash paging over it; an unknown size gets both.
#[tokio::test]
async fn a_file_path_is_paged_the_way_its_size_allows() {
    use crate::infrastructure::tools::filesystem::MAX_READ_BYTES;
    let tmp = tempfile::TempDir::new().unwrap();
    let small = tmp.path().join("small.log");
    std::fs::write(&small, "x\n").unwrap();
    let big = tmp.path().join("big.log");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(MAX_READ_BYTES + 1)
        .unwrap();
    let tool = RecallTool::new(recall_over(test_store_with_entry()), "s".to_string());
    let recall = |path: &std::path::Path| {
        let args = serde_json::json!({ "id": path }).to_string();
        let tool = &tool;
        async move { tool.execute(&args).await.unwrap().content }
    };
    let small_text = recall(&small).await;
    assert!(
        small_text.contains("so open it with read, e.g. {\"path\":"),
        "{small_text}"
    );
    assert!(!small_text.contains("sed -n"), "{small_text}");
    let big_text = recall(&big).await;
    let quoted = format!("'{}'", big.display());
    assert!(
        big_text.contains(&format!(
            "it is over read's 10.0MB limit, so page through it with bash, e.g. \
             sed -n '1,200p' {quoted} or tail -n 200 {quoted}"
        )),
        "{big_text}"
    );
    assert!(!big_text.contains("open it with read"), "{big_text}");
}
