use std::future::Future;
use std::pin::Pin;

use super::{error::DomainError, message::Message};

/// A conversation session identified by a unique key.
#[derive(Debug, Clone)]
pub struct Session {
    /// Unique key, e.g. "telegram:12345" or "cli:default".
    pub key: String,
    /// Ordered conversation history.
    pub messages: Vec<Message>,
}

impl Session {
    /// Create a new empty session.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            messages: vec![],
        }
    }

    /// Build a session key from channel and user ID.
    pub fn build_key(channel: &str, user_id: &str) -> String {
        format!("{}:{}", channel, user_id)
    }
}

/// Port: persistent storage for conversation sessions.
pub trait SessionStore: Send + Sync {
    /// Load a session by key. Returns None if no session exists.
    fn load(
        &self,
        key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Session>, DomainError>> + Send + '_>>;

    /// Save (create or update) a session.
    fn save(
        &self,
        session: &Session,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>>;

    /// Check if a session exists.
    fn exists(
        &self,
        key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DomainError>> + Send + '_>>;
}

/// A single spilled tool output entry.
#[derive(Debug, Clone)]
pub struct SpillEntry {
    pub id: String,
    pub tool: String,
    pub input_preview: String,
    pub tokens: usize,
    pub content: String,
}

/// Index-only view of spill entries (without full content).
#[derive(Debug, Clone)]
pub struct SpillIndex {
    pub id: String,
    pub tool: String,
    pub input_preview: String,
    pub tokens: usize,
}

/// Strip stale tool history from a session's messages while preserving conversation context.
///
/// Filtering rules (applied in order):
/// - `is_manifest == true`  → Drop (stale spill index referencing old tool names)
/// - `Role::Tool` where `tool_name == Some("recall")` → Keep (agent needs recall results)
/// - `Role::Tool` (any other tool) → Drop (stale tool result)
/// - `Role::Assistant` where any tool_call.name == "recall" → Keep entire message
/// - `Role::Assistant` with tool_calls, non-empty content → Keep, clear `tool_calls` vec
/// - `Role::Assistant` with tool_calls, empty content → Drop (pure dispatch, no text)
/// - `Role::User` / plain `Role::Assistant` → Keep always
pub fn strip_tool_history(messages: &[Message]) -> Vec<Message> {
    use super::message::Role;

    // First pass: identify assistant messages that pair with recall tool results.
    // An assistant message "pairs with recall" if any of its tool_calls has name "recall".
    let mut recall_assistant_indices = std::collections::HashSet::new();
    for (i, msg) in messages.iter().enumerate() {
        if msg.role == Role::Assistant && msg.tool_calls.iter().any(|tc| tc.name == "recall") {
            recall_assistant_indices.insert(i);
        }
    }

    let mut filtered = Vec::with_capacity(messages.len());
    for (i, msg) in messages.iter().enumerate() {
        match msg.role {
            // Drop stale spill manifests
            _ if msg.is_manifest => continue,

            // Keep recall tool results (recall output is conversational history)
            Role::Tool if msg.tool_name.as_deref() == Some("recall") => {
                filtered.push(msg.clone());
            }

            // Drop all other tool results
            Role::Tool => continue,

            // Keep assistant messages that pair with a recall tool call
            Role::Assistant if recall_assistant_indices.contains(&i) => {
                filtered.push(msg.clone());
            }

            // Assistant with tool calls but also text — keep text, clear tool calls
            Role::Assistant if !msg.tool_calls.is_empty() && !msg.content.is_empty() => {
                let mut kept = msg.clone();
                kept.tool_calls = vec![];
                filtered.push(kept);
            }

            // Assistant with tool calls but no text — pure dispatch, drop
            Role::Assistant if !msg.tool_calls.is_empty() => continue,

            // User and plain assistant messages — always keep
            _ => filtered.push(msg.clone()),
        }
    }
    filtered
}

/// Port: spill storage used by context pruning and recall().
pub trait ContextSpillStore: Send + Sync {
    fn append(
        &self,
        session_key: &str,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>>;

    fn recall(
        &self,
        session_key: &str,
        id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<SpillEntry>, DomainError>> + Send + '_>>;

    fn list_entries(
        &self,
        session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SpillIndex>, DomainError>> + Send + '_>>;

    /// Clear all spill entries for a session (e.g. on /reload).
    /// Truncates spill.jsonl to empty so the manifest rebuilds clean.
    fn clear(
        &self,
        session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::message::{Role, ToolCall};

    fn make_tool_call(name: &str) -> ToolCall {
        ToolCall {
            id: format!("id-{}", name),
            name: name.to_string(),
            arguments: "{}".to_string(),
        }
    }

    fn tool_msg(tool_name: &str, content: &str) -> Message {
        let mut m = Message::tool("some-id", content);
        m.tool_name = Some(tool_name.to_string());
        m
    }

    fn manifest_msg() -> Message {
        let mut m = Message::assistant("[spill manifest]", vec![]);
        m.is_manifest = true;
        m
    }

    #[test]
    fn test_strip_keeps_user_and_plain_assistant() {
        let messages = vec![
            Message::user("hello"),
            Message::assistant("sure thing", vec![]),
        ];
        let filtered = strip_tool_history(&messages);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].role, Role::User);
        assert_eq!(filtered[0].content, "hello");
        assert_eq!(filtered[1].role, Role::Assistant);
        assert_eq!(filtered[1].content, "sure thing");
    }

    #[test]
    fn test_strip_drops_manifest() {
        let messages = vec![Message::user("hello"), manifest_msg()];
        let filtered = strip_tool_history(&messages);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].role, Role::User);
    }

    #[test]
    fn test_strip_drops_non_recall_tool_results() {
        let messages = vec![
            Message::user("do something"),
            Message::assistant("", vec![make_tool_call("exec")]),
            tool_msg("exec", "exec output"),
        ];
        let filtered = strip_tool_history(&messages);
        // Only the user message survives (assistant has empty content + tool_call → dropped)
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].role, Role::User);
    }

    #[test]
    fn test_strip_keeps_recall_tool_results_and_paired_assistant() {
        let messages = vec![
            Message::user("what did we do?"),
            Message::assistant("(calls recall)", vec![make_tool_call("recall")]),
            tool_msg("recall", "recalled content"),
        ];
        let filtered = strip_tool_history(&messages);
        assert_eq!(filtered.len(), 3);
        assert_eq!(filtered[0].content, "what did we do?");
        assert_eq!(filtered[1].content, "(calls recall)");
        assert_eq!(filtered[2].content, "recalled content");
    }

    #[test]
    fn test_strip_preserves_narrative_text_from_mixed_assistant() {
        let messages = vec![
            Message::user("do something"),
            Message::assistant("I will run exec", vec![make_tool_call("exec")]),
            tool_msg("exec", "exec output"),
        ];
        let filtered = strip_tool_history(&messages);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].role, Role::User);
        assert_eq!(filtered[1].role, Role::Assistant);
        assert_eq!(filtered[1].content, "I will run exec");
        // tool_calls must be cleared
        assert!(filtered[1].tool_calls.is_empty());
    }

    #[test]
    fn test_strip_drops_pure_dispatch_assistant_with_no_text() {
        let messages = vec![
            Message::user("run it"),
            Message::assistant("", vec![make_tool_call("exec")]),
            tool_msg("exec", "tool result"),
        ];
        let filtered = strip_tool_history(&messages);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].role, Role::User);
    }

    #[test]
    fn test_strip_empty_messages() {
        let messages: Vec<Message> = vec![];
        let filtered = strip_tool_history(&messages);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_strip_no_tool_messages_unchanged() {
        let messages = vec![
            Message::user("first"),
            Message::assistant("second", vec![]),
            Message::user("third"),
        ];
        let filtered = strip_tool_history(&messages);
        assert_eq!(filtered.len(), 3);
    }
}
