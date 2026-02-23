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
    session_key: String,
    /// Tracks recall counts per ID for diagnostic warnings.
    recall_counts: Mutex<HashMap<String, u32>>,
}

impl RecallTool {
    pub fn new(spill_store: Arc<dyn ContextSpillStore>, session_key: String) -> Self {
        Self {
            spill_store,
            session_key,
            recall_counts: Mutex::new(HashMap::new()),
        }
    }
}

impl std::fmt::Debug for RecallTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecallTool")
            .field("session_key", &self.session_key)
            .finish()
    }
}

impl Tool for RecallTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "recall".to_string(),
            description: "Retrieve a previously collapsed tool output by its ID. \
                Use the ID shown in collapse stubs like: recall(\"turn20:bash:0\"). \
                Use recall(\"list\") for the full index."
                .to_string(),
            parameters_schema: r#"{"type":"object","properties":{"id":{"type":"string","description":"The spill ID from the collapse stub, or \"list\" for the full index"}},"required":["id"]}"#.to_string(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let id = extract_id(arguments);
        Box::pin(async move {
            if id == "list" {
                return self.handle_list().await;
            }

            // Track recall count for diagnostics
            {
                let mut counts = self.recall_counts.lock().unwrap();
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

            match self.spill_store.recall(&self.session_key, &id).await? {
                Some(entry) => Ok(ToolResult {
                    content: entry.content,
                    is_error: false,
                }),
                None => Ok(ToolResult {
                    content: format!("No spilled output found for id: {}", id),
                    is_error: true,
                }),
            }
        })
    }
}

impl RecallTool {
    async fn handle_list(&self) -> Result<ToolResult, DomainError> {
        let entries = self.spill_store.list_entries(&self.session_key).await?;

        if entries.is_empty() {
            return Ok(ToolResult {
                content: "No spilled outputs in this session.".to_string(),
                is_error: false,
            });
        }

        let mut output = format!("Spilled outputs ({} entries):\n", entries.len());
        for entry in &entries {
            output.push_str(&format!(
                "  {} — {} ({} tokens)\n",
                entry.id, entry.input_preview, entry.tokens
            ));
        }
        Ok(ToolResult {
            content: output,
            is_error: false,
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
        ) -> Pin<Box<dyn Future<Output = Result<Vec<SpillIndex>, DomainError>> + Send + '_>>
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
            Box::pin(async move { Ok(entries) })
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
        assert!(def.description.contains("collapsed tool output"));
    }
}
