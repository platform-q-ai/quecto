// RecallTool: the `recall` tool's adapter over the sessions capability's
// recall use case (D9 #1978). The model's main pointer is a watermark cut's
// stub (#2403), naming `recall("archive")` (then `archive:2`...), which
// lists what the cut archived, each with its id; a rare emergency-ladder
// stub names its id inline (`recall("turn20:bash:0")`).
//
// This adapter parses the tool's argument schema, formats the use case's
// outcomes as the tool result the model has always seen, and keeps its
// repeated-recall diagnostics. It selects nothing: `list` versus one entry,
// a missing entry and a malformed id are the use case's decisions
// (`RecallContext`, `RecallQuery`).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::application::sessions::dto::retained_context::{
    RecallError, RecallOutcome, RecallQuery,
};
use crate::application::sessions::use_cases::RecallContext;
use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::sessions::entities::session_identity::SessionIdentity;
use crate::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};

mod results;
#[cfg(test)]
use results::looks_like_file_path;
use results::{index_result, not_found};

/// Tool that retrieves previously collapsed tool outputs by their spill ID.
pub struct RecallTool {
    recall: Arc<RecallContext>,
    session_key: Mutex<SessionIdentity>,
    /// Tracks recall counts per ID for diagnostic warnings.
    recall_counts: Mutex<HashMap<String, u32>>,
}

impl RecallTool {
    pub fn new(recall: Arc<RecallContext>, session_key: String) -> Self {
        Self {
            recall,
            session_key: Mutex::new(SessionIdentity::from_persisted_key(session_key)),
            recall_counts: Mutex::new(HashMap::new()),
        }
    }
}

impl std::fmt::Debug for RecallTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecallTool")
            .field(
                "session_key",
                &self
                    .session_key
                    .lock()
                    .ok()
                    .map(|key| key.runtime_key().to_string()),
            )
            .finish()
    }
}

impl Tool for RecallTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "recall".into(),
            description: "Retrieve spilled session memory by ID. \
                A context archive stub names its index (recall(\"archive\"), then \
                \"archive:2\"...), which lists the archived messages with their IDs. \
                Use recall(\"list\") for the full session-memory index, then pass an ID \
                from it to retrieve its content. A ladder stub shows its ID inline, for \
                example: recall(\"turn20:bash:0\")."
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"id":{"type":"string","description":"An ID from an archive index, a stub or the list, or \"list\" for the full index"}},"required":["id"]}"#.into(),
        }
    }

    fn set_session_key(&self, session_key: String) {
        *self.session_key.lock().unwrap() = SessionIdentity::from_persisted_key(session_key);
        self.recall_counts.lock().unwrap().clear();
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let id = extract_id(arguments);
        let session_key = self.session_key.lock().unwrap().clone();
        Box::pin(async move {
            let query = match RecallQuery::parse(&id) {
                Ok(query) => query,
                Err(RecallError::MalformedId) => return Ok(not_found(&id).await),
                Err(RecallError::Store(error)) => return Err(error),
            };
            if matches!(query, RecallQuery::Entry(_)) {
                self.note_recall(&id);
            }
            match self.recall.recall(&session_key, &query).await {
                Ok(RecallOutcome::Index(entries)) => Ok(index_result(&entries)),
                Ok(RecallOutcome::Entry(entry)) => Ok(ToolResult {
                    content: entry.recalled_text(),
                    is_error: false,
                    image_blocks: vec![],
                    delivery_metadata: None,
                }),
                Ok(RecallOutcome::Missing(_)) => Ok(not_found(&id).await),
                Err(RecallError::MalformedId) => Ok(not_found(&id).await),
                Err(RecallError::Store(error)) => Err(error),
            }
        })
    }
}

impl RecallTool {
    /// Track recall count for diagnostics (capped to prevent unbounded
    /// growth): a third recall of the same id warns that the model may be
    /// stuck in a recall-collapse loop.
    fn note_recall(&self, id: &str) {
        let mut counts = self.recall_counts.lock().unwrap();
        // Cap at 256 tracked IDs to prevent memory leak in long sessions
        if counts.len() < 256 || counts.contains_key(id) {
            let count = counts.entry(id.to_string()).or_insert(0);
            *count += 1;
            if *count >= 3 {
                tracing::warn!(
                    target: "context_prune",
                    id = id,
                    recall_count = *count,
                    "repeated recall — model may be stuck in a recall-collapse loop"
                );
            }
        }
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
