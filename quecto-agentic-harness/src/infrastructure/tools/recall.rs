// RecallTool: retrieves spilled tool outputs by ID.
//
// The model sees collapse stubs like:
//   [bash: find ~/.local -type d (19156 tokens) — recall("turn20:bash:0")]
// and can call this tool to retrieve the full output.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::domain::error::DomainError;
use crate::domain::session::ContextSpillStore;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

/// Tool that retrieves previously collapsed tool outputs by their spill ID.
pub struct RecallTool {
    spill_store: Arc<dyn ContextSpillStore>,
    session_key: Mutex<String>,
    /// Tracks recall counts per ID for diagnostic warnings.
    recall_counts: Mutex<HashMap<String, u32>>,
}

impl RecallTool {
    pub fn new(spill_store: Arc<dyn ContextSpillStore>, session_key: String) -> Self {
        Self {
            spill_store,
            session_key: Mutex::new(session_key),
            recall_counts: Mutex::new(HashMap::new()),
        }
    }
}

impl std::fmt::Debug for RecallTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecallTool")
            .field("session_key", &self.session_key.lock().ok().as_deref())
            .finish()
    }
}

impl Tool for RecallTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "recall".into(),
            description: "Retrieve spilled session memory by ID. \
                Use recall(\"list\") for the full session-memory index, then pass an ID \
                from that result to retrieve its content. Collapse stubs also show IDs \
                inline, for example: recall(\"turn20:bash:0\")."
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"id":{"type":"string","description":"The spill ID from the collapse stub, or \"list\" for the full index"}},"required":["id"]}"#.into(),
        }
    }

    fn set_session_key(&self, session_key: String) {
        *self.session_key.lock().unwrap() = session_key;
        self.recall_counts.lock().unwrap().clear();
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let id = extract_id(arguments);
        let session_key = self.session_key.lock().unwrap().clone();
        Box::pin(async move {
            if id == "list" {
                return self.handle_list().await;
            }

            // Track recall count for diagnostics (capped to prevent unbounded growth)
            {
                let mut counts = self.recall_counts.lock().unwrap();
                // Cap at 256 tracked IDs to prevent memory leak in long sessions
                if counts.len() < 256 || counts.contains_key(&id) {
                    let count = counts.entry(id.clone()).or_insert(0);
                    *count += 1;
                    if *count >= 3 {
                        tracing::warn!(
                            target: "context_prune",
                            id = id.as_str(),
                            recall_count = *count,
                            "repeated recall — model may be stuck in a recall-collapse loop"
                        );
                    }
                }
            }

            match self.spill_store.recall(&session_key, &id).await? {
                Some(entry) => Ok(ToolResult {
                    content: entry.content,
                    is_error: false,
                    image_blocks: vec![],
                }),
                None => Ok(ToolResult {
                    content: format!("No spilled output found for id: {}", id),
                    is_error: true,
                    image_blocks: vec![],
                }),
            }
        })
    }
}

impl RecallTool {
    async fn handle_list(&self) -> Result<ToolResult, DomainError> {
        let session_key = self.session_key.lock().unwrap().clone();
        let entries = self.spill_store.list_entries(&session_key).await?;

        if entries.is_empty() {
            return Ok(ToolResult {
                content: "No spilled outputs in this session.".to_string(),
                is_error: false,
                image_blocks: vec![],
            });
        }

        let mut output = format!("Spilled outputs ({} entries):\n", entries.len());
        for entry in entries.iter() {
            output.push_str(&format!(
                "  {} — {} ({} tokens)\n",
                entry.id, entry.input_preview, entry.tokens
            ));
        }
        Ok(ToolResult {
            content: output,
            is_error: false,
            image_blocks: vec![],
        })
    }
}

/// Extract the "id" field from JSON arguments.
fn extract_id(arguments: &str) -> String {
    serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|v| v.get("id").and_then(|v| v.as_str()).map(String::from))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
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
        ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>>
        {
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
        ) -> Pin<Box<dyn Future<Output = Result<Arc<Vec<SpillIndex>>, DomainError>> + Send + '_>>
        {
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
        ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>>
        {
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
        ) -> Pin<Box<dyn Future<Output = Result<Arc<Vec<SpillIndex>>, DomainError>> + Send + '_>>
        {
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
        let store = test_store_with_entry();
        let tool = RecallTool::new(store, "test-session".to_string());
        let result = tool.execute(r#"{"id":"turn5:bash:0"}"#).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content, "hello world output");
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
}
