use std::collections::HashSet;
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

    // Single-pass filter: an assistant message that calls "recall" is kept in full;
    // all other tool-dispatching assistant messages are either text-preserved or dropped.
    // Tool results are kept only for "recall"; all others (and manifests) are dropped.
    let mut filtered = Vec::with_capacity(messages.len());
    for msg in messages {
        match msg.role {
            // Drop stale spill manifests
            _ if msg.is_manifest => continue,

            // Keep recall tool results (agent needs them to access conversation history)
            Role::Tool if msg.tool_name.as_deref() == Some("recall") => {
                filtered.push(msg.clone());
            }

            // Drop all other tool results
            Role::Tool => continue,

            Role::Assistant if !msg.tool_calls.is_empty() => {
                // Keep entire assistant message if it calls "recall" (must not orphan the result)
                if msg.tool_calls.iter().any(|tc| tc.name == "recall") {
                    filtered.push(msg.clone());
                } else if !msg.content.is_empty() {
                    // Non-recall tool call with narrative text: keep text, clear tool_calls
                    let mut kept = msg.clone();
                    kept.tool_calls = vec![];
                    filtered.push(kept);
                }
                // else: pure dispatch (no text) — drop
            }

            // User and plain assistant messages — always keep
            _ => filtered.push(msg.clone()),
        }
    }
    filtered
}

/// Diagnostic information returned by [`filter_orphan_tool_pairs`].
///
/// Callers (infrastructure providers) are responsible for logging these with
/// provider-specific context. Domain functions must remain side-effect-free.
#[derive(Debug, Default)]
pub struct OrphanDiag {
    /// Tool-call IDs present in assistant messages but missing a tool result.
    pub orphaned_calls: Vec<String>,
    /// Tool-call IDs present in tool results but missing an assistant call.
    pub orphaned_results: Vec<String>,
}

impl OrphanDiag {
    /// Returns `true` if any orphaned IDs were found.
    pub fn has_orphans(&self) -> bool {
        !self.orphaned_calls.is_empty() || !self.orphaned_results.is_empty()
    }
}

/// Return the set of tool-call IDs that have a matching pair on both the
/// assistant side (tool_calls) and the tool side (tool_call_id), together
/// with diagnostic info about any orphaned IDs.
///
/// Orphaned IDs — calls without a result or results without a call — are
/// excluded from the returned set. Callers should filter their message list
/// to only emit tool calls / results whose ID appears in the valid set,
/// preventing provider API errors (e.g. HTTP 400) from mismatched pairs.
///
/// Extracted from `CodexProvider::valid_call_id_pairs` (#311) so all
/// providers benefit from the same logic. The caller is responsible for
/// logging `OrphanDiag` with provider-specific context — domain functions
/// must remain pure (no I/O or side-effects).
///
/// # Allocation note
///
/// Single-pass partition over the `sent` set: each ID is classified as
/// valid or orphaned in one drain. Empty `Vec`s in `OrphanDiag` do not
/// heap-allocate (Rust `Vec::new()` has zero capacity), so the happy
/// path (no orphans) is effectively zero-alloc beyond the two `HashSet`s.
pub fn filter_orphan_tool_pairs(messages: &[Message]) -> (HashSet<String>, OrphanDiag) {
    use super::message::Role;

    let mut sent: HashSet<String> = HashSet::new();
    let mut received: HashSet<String> = HashSet::new();

    for msg in messages {
        match msg.role {
            Role::Assistant => {
                for tc in &msg.tool_calls {
                    sent.insert(tc.id.clone());
                }
            }
            Role::Tool => {
                if let Some(ref cid) = msg.tool_call_id {
                    received.insert(cid.clone());
                }
            }
            _ => {}
        }
    }

    // Single-pass partition: split `sent` into valid (matched) and orphaned in one drain.
    let mut valid = HashSet::new();
    let mut orphaned_calls = Vec::new();
    for id in sent {
        if received.contains(&id) {
            valid.insert(id);
        } else {
            orphaned_calls.push(id);
        }
    }
    let orphaned_results: Vec<String> = received
        .into_iter()
        .filter(|id| !valid.contains(id))
        .collect();

    let diag = OrphanDiag {
        orphaned_calls,
        orphaned_results,
    };
    (valid, diag)
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
            Message::assistant("", vec![make_tool_call("bash")]),
            tool_msg("bash", "bash output"),
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
            Message::assistant("I will run bash", vec![make_tool_call("bash")]),
            tool_msg("bash", "bash output"),
        ];
        let filtered = strip_tool_history(&messages);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].role, Role::User);
        assert_eq!(filtered[1].role, Role::Assistant);
        assert_eq!(filtered[1].content, "I will run bash");
        // tool_calls must be cleared
        assert!(filtered[1].tool_calls.is_empty());
    }

    #[test]
    fn test_strip_drops_pure_dispatch_assistant_with_no_text() {
        let messages = vec![
            Message::user("run it"),
            Message::assistant("", vec![make_tool_call("bash")]),
            tool_msg("bash", "tool result"),
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

    // --- #311: filter_orphan_tool_pairs domain function ---

    #[test]
    fn filter_orphan_pairs_matched_calls_preserved() {
        let mut assistant = Message::assistant("", vec![make_tool_call("bash")]);
        assistant.tool_calls[0].id = "call-1".to_string();
        let mut tool_result = Message::tool("call-1", "output");
        tool_result.tool_call_id = Some("call-1".to_string());
        let messages = vec![assistant, tool_result];
        let (valid, diag) = filter_orphan_tool_pairs(&messages);
        assert!(valid.contains("call-1"));
        assert!(!diag.has_orphans());
    }

    #[test]
    fn filter_orphan_pairs_unmatched_call_excluded() {
        let mut assistant = Message::assistant("", vec![make_tool_call("bash")]);
        assistant.tool_calls[0].id = "call-orphan".to_string();
        // No matching tool result
        let messages = vec![assistant];
        let (valid, diag) = filter_orphan_tool_pairs(&messages);
        assert!(!valid.contains("call-orphan"));
        assert!(diag.has_orphans());
        assert!(diag.orphaned_calls.contains(&"call-orphan".to_string()));
    }

    #[test]
    fn filter_orphan_pairs_unmatched_result_excluded() {
        let mut tool_result = Message::tool("ghost-id", "output");
        tool_result.tool_call_id = Some("ghost-id".to_string());
        // No matching assistant call
        let messages = vec![tool_result];
        let (valid, diag) = filter_orphan_tool_pairs(&messages);
        assert!(!valid.contains("ghost-id"));
        assert!(diag.has_orphans());
        assert!(diag.orphaned_results.contains(&"ghost-id".to_string()));
    }

    #[test]
    fn filter_orphan_pairs_empty_messages_returns_empty() {
        let messages: Vec<Message> = vec![];
        let (valid, diag) = filter_orphan_tool_pairs(&messages);
        assert!(valid.is_empty());
        assert!(!diag.has_orphans());
    }
}
