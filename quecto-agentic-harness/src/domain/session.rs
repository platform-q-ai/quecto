use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use super::{error::DomainError, message::Message};

pub type SpillEntries = Arc<Vec<SpillIndex>>;
pub type SpillIndexList<'a> =
    Pin<Box<dyn Future<Output = Result<SpillEntries, DomainError>> + Send + 'a>>;
pub type SpillPresence<'a> = Pin<Box<dyn Future<Output = Result<bool, DomainError>> + Send + 'a>>;

/// Prefix for user-facing interactive chat sessions.
pub const USER_CHAT_PREFIX: &str = "chat-";

/// Build a user-chat session key from a timestamp and a uniqueness token.
///
/// Pure: the interface layer supplies `secs` (wall clock) and a `uniq` value
/// that is distinct across concurrent launches (e.g. PID combined with a
/// per-process counter), so two sessions started in the same second never
/// collide on a key.
pub fn user_chat_key(secs: u64, uniq: u64) -> String {
    format!("{USER_CHAT_PREFIX}{secs}-{uniq:x}")
}

/// Lightweight metadata for a persisted conversation session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    /// Unique key, e.g. "cli:default".
    pub key: String,
    /// Raw title datum — the session's first user message, trimmed (empty when
    /// none). Presentation (truncation, "(untitled)") is applied by the display
    /// layer, not by persistence.
    pub title: String,
    /// Number of persisted user/assistant messages.
    pub message_count: usize,
    /// Last modification time in Unix seconds, when available.
    pub updated_unix_secs: Option<u64>,
}

/// Cross-process liveness of a persisted sub-agent roster entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentLiveness {
    Live,
    Detached,
    #[default]
    Dead,
}

/// Why a persisted sub-agent roster row may be restored on session resume.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentRestoreReason {
    /// Legacy rows omitted the field. Only verified live rows may restore.
    #[default]
    LegacyUnspecified,
    /// Ordinary TUI exit intentionally stopped this previously live row.
    OrdinaryTuiExitStopped,
    /// The user explicitly killed this row before ordinary TUI exit.
    ExplicitlyKilled,
    /// Forward-compatible safe default for unknown explicit values.
    #[serde(other)]
    Unknown,
}

/// Pending default get_messages delivery awaiting parent-context acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingMessageReport {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub receipt: String,
    #[serde(default)]
    pub response: String,
    pub ordinal: u64,
}

/// Durable, read-only summary of a sub-agent spawned by this session.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedSubagentRosterEntry {
    #[serde(default)]
    pub agent_uuid: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub session_key: String,
    #[serde(default)]
    pub socket_path: PathBuf,
    #[serde(default)]
    pub pid: u32,
    #[serde(default)]
    pub liveness: SubagentLiveness,
    #[serde(default)]
    pub restore_reason: SubagentRestoreReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_message_ordinal: Option<u64>,
    #[serde(default, skip_serializing_if = "std::collections::VecDeque::is_empty")]
    pub pending_message_reports: std::collections::VecDeque<PendingMessageReport>,
}

/// A conversation session identified by a unique key.
#[derive(Debug, Clone)]
pub struct Session {
    /// Unique key, e.g. "telegram:12345" or "cli:default".
    pub key: String,
    /// Ordered conversation history.
    pub messages: Vec<Message>,
    /// Optional persisted workflow run for UDS-native workflow sessions.
    pub workflow_run: Option<super::workflow::WorkflowRunPersisted>,
    /// Persisted sub-agent roster for resumed masters (#1461).
    pub subagent_roster: Vec<PersistedSubagentRosterEntry>,
}

impl Session {
    /// Create a new empty session.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            messages: vec![],
            workflow_run: None,
            subagent_roster: Vec::new(),
        }
    }

    /// Build a session key from channel and user ID.
    pub fn build_key(channel: &str, user_id: &str) -> String {
        format!("{}:{}", channel, user_id)
    }
}

/// Port: persistent storage for conversation sessions.
pub trait SessionStore: Send + Sync {
    /// Claim single-writer ownership of `key` before opening or resuming it
    /// for writing (#1460): a key owned by another live process must be
    /// refused HERE, at open time, not only when the first turn is saved —
    /// otherwise a whole paid turn can run before the conflict surfaces.
    /// Default is a no-op for stores without cross-process shared state.
    fn claim(&self, _key: &str) -> Result<(), DomainError> {
        Ok(())
    }

    /// Release this process's ownership claim when a live session switches
    /// away from a key. Stores without explicit ownership can ignore this.
    fn release(&self, _key: &str) {}

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

    /// Save a session when the caller knows how many messages are already durable.
    fn save_delta<'a>(
        &'a self,
        key: &'a str,
        messages: &'a [Message],
        _previously_persisted: usize,
        workflow_run: Option<super::workflow::WorkflowRunPersisted>,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        let session = Session {
            key: key.to_string(),
            messages: messages.to_vec(),
            workflow_run,
            subagent_roster: Vec::new(),
        };
        Box::pin(async move { self.save(&session).await })
    }

    /// Save a delta when the caller guarantees the durable prefix is unchanged.
    /// Adapters may use this stronger contract to avoid reading that prefix.
    fn save_clean_delta<'a>(
        &'a self,
        key: &'a str,
        messages: &'a [Message],
        previously_persisted: usize,
        workflow_run: Option<super::workflow::WorkflowRunPersisted>,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.save_delta(key, messages, previously_persisted, workflow_run)
    }

    /// Check if a session exists.
    fn exists(
        &self,
        key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DomainError>> + Send + '_>>;

    /// List persisted sessions, newest first when modification times are
    /// available. When `key_prefix` is `Some`, only sessions whose key starts
    /// with it are returned; the caller supplies this policy and the adapter
    /// uses it to skip non-matching files cheaply (without reading/parsing them).
    ///
    /// This is a SUMMARY-ONLY view and is NOT a load guarantee: an
    /// implementation may derive summaries from a lightweight projection of
    /// each session and therefore surface entries whose full bodies are
    /// malformed. A returned [`SessionSummary`] does not guarantee that the
    /// corresponding [`Self::load`] will succeed — callers that open a listed
    /// session must handle a subsequent load failure gracefully.
    fn list(
        &self,
        key_prefix: Option<&str>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SessionSummary>, DomainError>> + Send + '_>>;
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
                    kept.invalidate_token_cache();
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
    let mut valid = HashSet::with_capacity(sent.len());
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

    fn list_entries(&self, session_key: &str) -> SpillIndexList<'_>;

    /// Return whether any spill entry exists without requiring callers to
    /// materialize the complete index. Stores may override this with a cheap
    /// metadata check; the default preserves compatibility for simple stores.
    fn has_entries<'a>(&'a self, session_key: &'a str) -> SpillPresence<'a> {
        Box::pin(async move { Ok(!self.list_entries(session_key).await?.is_empty()) })
    }

    /// Clear all spill entries for a session (e.g. on /reload).
    /// Truncates spill.jsonl to empty so the manifest rebuilds clean.
    fn clear(
        &self,
        session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>>;
}

#[cfg(test)]
#[path = "session_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
