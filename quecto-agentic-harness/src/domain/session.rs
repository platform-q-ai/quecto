//! Pure session vocabulary: keys, summaries, the persisted roster and the
//! conversation-history policies. The typed identity is
//! [`super::session_identity::SessionIdentity`]; the persistence ports
//! (`SessionStore`, `ContextSpillStore`) are the application's
//! (`application::sessions::ports`, #1960).
use std::collections::HashSet;
use std::sync::Arc;

use super::message::Message;
use super::session_identity::SessionIdentity;

pub type SpillEntries = Arc<Vec<SpillIndex>>;

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

/// Why a persisted sub-agent roster row was written. Restore never turns a
/// row into an operational child (#1937); the reason is retained as history
/// and for reading legacy records.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentRestoreReason {
    /// Legacy rows omitted the field.
    #[default]
    LegacyUnspecified,
    /// Ordinary TUI exit record (old clients also used this for detach).
    OrdinaryTuiExitStopped,
    /// The user explicitly killed this row before ordinary TUI exit.
    ExplicitlyKilled,
    /// Forward-compatible safe default for unknown explicit values.
    #[serde(other)]
    Unknown,
}

impl SubagentRestoreReason {
    pub const ORDINARY_TUI_EXIT_STOPPED_WIRE: &'static str = "ordinary_tui_exit_stopped";
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

/// Durable history metadata for a sub-agent spawned by this session.
///
/// This is not an operational row and never becomes one (#1937): a
/// launcher-created child is lifetime-scoped to the harness that launched it,
/// so nothing it describes can be alive when the session is restored. Restore
/// keeps the transcript, workflow and past child messages and creates no
/// child row from these records; the master re-spawns the workers it needs.
/// Legacy records carried the child's `socketPath` and `pid` as recovery
/// authority; the reader ignores those fields (serde skips unknown fields)
/// and the next save migrates the record without them.
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

/// A conversation session identified by its typed identity.
#[derive(Debug, Clone)]
pub struct Session {
    /// The session's identity: its unique key, e.g. "telegram:12345" or
    /// "cli:default" (#1970).
    pub key: SessionIdentity,
    /// Ordered conversation history.
    pub messages: Vec<Message>,
    /// Optional persisted workflow run for UDS-native workflow sessions.
    pub workflow_run: Option<super::workflow::WorkflowRunPersisted>,
    /// Persisted sub-agent roster for resumed masters (#1461).
    pub subagent_roster: Vec<PersistedSubagentRosterEntry>,
}

impl Session {
    /// Create a new empty session.
    pub fn new(key: SessionIdentity) -> Self {
        Self {
            key,
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

/// Give every message of a persisted conversation a durable ordinal
/// (#1586): existing ordinals are kept, the missing ones are assigned in
/// conversation order strictly above the largest ordinal already present,
/// so ordinals stay unique and monotonic across prunes and resumes.
pub fn assign_missing_ordinals(messages: &mut [Message]) {
    let mut next = messages
        .iter()
        .filter_map(|message| message.ordinal)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    for message in messages {
        if message.ordinal.is_none() {
            message.ordinal = Some(next);
            next = next.saturating_add(1);
        }
    }
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

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
