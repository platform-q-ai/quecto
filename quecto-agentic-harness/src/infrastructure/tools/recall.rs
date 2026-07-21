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
#[path = "recall_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "recall_tests.rs"]
mod tests;
