//! Edit-side vocabulary of an ordered conversation (#1864, #1865): the two
//! history-replacing edits a user may ask for — clearing the transcript
//! and rewinding it to a user-message boundary — and how a rewind target
//! named on the wire is resolved against the full conversation.
//!
//! Pure rules over [`Message`] values. The sessions use cases sequence
//! them with the ledger, retention and persistence effects; nothing here
//! performs an effect or knows a wire field.
use super::conversation_view::position_by_id;
use super::ids::MessageId;
use super::message::{Message, Role};

/// Clear the conversation, preserving only a leading injected system
/// prompt (a non-manifest system message); a retention manifest at the
/// head is history and goes with the rest.
pub fn clear_conversation(messages: &mut Vec<Message>) {
    let keep = messages
        .first()
        .is_some_and(|m| m.role == Role::System && !m.is_manifest);
    if keep {
        messages.truncate(1);
    } else {
        messages.clear();
    }
}

/// Which message a rewind removes from: the stable id every client since
/// #1061 names, or the legacy absolute index (#1059) a pre-paging client
/// still sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewindTarget {
    MessageId(MessageId),
    LegacyIndex(usize),
}

impl RewindTarget {
    /// The target a request names, preferring the stable id whenever one
    /// is given; `None` when neither is.
    pub fn select(message_id: Option<MessageId>, legacy_index: Option<usize>) -> Option<Self> {
        match (message_id, legacy_index) {
            (Some(id), _) => Some(Self::MessageId(id)),
            (None, Some(index)) => Some(Self::LegacyIndex(index)),
            (None, None) => None,
        }
    }
}

/// Why a rewind target names no removable boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewindTargetError {
    /// The stable id names no message of the conversation: stale (already
    /// rewound or cleared away) or never issued.
    NotFound,
    /// A legacy index against a conversation longer than one history page:
    /// a paging client's index is page-local, so applying it absolutely
    /// would truncate a much older turn. Refused; the id is required.
    AmbiguousLegacyIndex,
}

/// Resolve a rewind target to an absolute index into `messages`. A stable
/// id is resolved against the full conversation; a legacy index is
/// honoured only while the conversation fits in one page of
/// `legacy_index_window` messages (#1061 review follow-up).
pub fn resolve_rewind_target(
    messages: &[Message],
    target: &RewindTarget,
    legacy_index_window: usize,
) -> Result<usize, RewindTargetError> {
    match target {
        RewindTarget::MessageId(id) => {
            position_by_id(messages, id).ok_or(RewindTargetError::NotFound)
        }
        RewindTarget::LegacyIndex(_) if messages.len() > legacy_index_window => {
            Err(RewindTargetError::AmbiguousLegacyIndex)
        }
        RewindTarget::LegacyIndex(index) => Ok(*index),
    }
}

/// Truncate the conversation at a user-message boundary: the user message
/// at `index` and everything after it are removed, preserving earlier
/// system prompts and completed turns. Nothing changes and `false` is
/// returned when `index` is out of range or names a non-user message.
pub fn truncate_at_user_message(messages: &mut Vec<Message>, index: usize) -> bool {
    let Some(message) = messages.get(index) else {
        return false;
    };
    if message.role != Role::User {
        return false;
    }
    messages.truncate(index);
    true
}

#[cfg(test)]
#[path = "conversation_edit_tests.rs"]
mod tests;
