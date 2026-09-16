//! Boundary values of Rewind a conversation (#1865, D6 #1975): what a
//! caller asks for, what the transaction did, and how it failed — before
//! any change (a refused target) or after the history was replaced (a
//! failed save).
use super::SaveSessionError;
use crate::application::sessions::conversation_ledger::LedgerAdvance;
use crate::domain::conversation_edit::{RewindTarget, RewindTargetError};

/// Which user message to rewind to, and how long a conversation may be
/// for a legacy index to still be unambiguous (the protocol's history page
/// size, decided by the transport).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindRequest {
    pub target: RewindTarget,
    pub legacy_index_window: usize,
}

/// The conversation was rewound: the user message at `message_index` and
/// everything after it are gone from the live transcript, the ledger and
/// the retention namespace (to the degree the best-effort clear
/// succeeded); the accounting and pending queue were reset and the
/// session was saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewoundConversation {
    /// The absolute index the target resolved to.
    pub message_index: usize,
    /// The ledger position after the rewind, for the transports to announce.
    pub ledger: LedgerAdvance,
}

/// Why a rewind did not complete. The first four refuse before anything
/// changes; a `Save` failure comes after the history, ledger, accounting
/// and pending queue were already rewound and stay so — `ledger` carries
/// the position the transports must still announce.
#[derive(Debug)]
pub enum RewindConversationError {
    /// Neither a stable id nor a legacy index was given.
    MissingTarget,
    /// The stable id names no message of the conversation.
    TargetNotFound,
    /// A legacy index against a conversation longer than one page.
    AmbiguousLegacyIndex,
    /// The resolved index is out of range or not a user message.
    InvalidTarget,
    Save {
        ledger: LedgerAdvance,
        error: SaveSessionError,
    },
}

impl RewindConversationError {
    /// The ledger position a failed transaction still advanced to; `None`
    /// when the rewind was refused before any change.
    pub fn ledger(&self) -> Option<LedgerAdvance> {
        match self {
            Self::Save { ledger, .. } => Some(*ledger),
            _ => None,
        }
    }
}

impl From<RewindTargetError> for RewindConversationError {
    fn from(err: RewindTargetError) -> Self {
        match err {
            RewindTargetError::NotFound => Self::TargetNotFound,
            RewindTargetError::AmbiguousLegacyIndex => Self::AmbiguousLegacyIndex,
        }
    }
}

impl std::fmt::Display for RewindConversationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingTarget => f.write_str("rewind requires messageId or messageIndex"),
            Self::TargetNotFound => f.write_str("rewind target not found"),
            Self::AmbiguousLegacyIndex => f.write_str(
                "messageIndex is ambiguous beyond one history page; rewind requires messageId",
            ),
            Self::InvalidTarget => f.write_str("invalid rewind target"),
            Self::Save { error, .. } => write!(f, "failed to save rewound session: {error}"),
        }
    }
}

impl std::error::Error for RewindConversationError {}

#[cfg(test)]
#[path = "rewind_conversation_tests.rs"]
mod tests;
