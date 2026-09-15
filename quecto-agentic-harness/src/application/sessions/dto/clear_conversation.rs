//! Boundary values of Clear conversation history (#1864, D6 #1975): what
//! the transaction did, and how it failed after the history was replaced.
use super::SaveSessionError;
use crate::application::sessions::conversation_ledger::LedgerAdvance;

/// The conversation was cleared: the live transcript holds at most the
/// injected system prompt, the ledger opened a new epoch, the accounting
/// and pending queue were reset, and the session was saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClearedConversation {
    /// The ledger position after the clear, for the transports to announce.
    pub ledger: LedgerAdvance,
}

/// Why a clear did not complete. The transaction is intentionally not
/// atomic: by the time a save fails the history, ledger, accounting and
/// pending queue are already cleared and stay so — `ledger` carries the
/// position the transports must still announce.
#[derive(Debug)]
pub enum ClearConversationError {
    Save {
        ledger: LedgerAdvance,
        error: SaveSessionError,
    },
}

impl ClearConversationError {
    /// The ledger position the failed transaction still advanced to.
    pub fn ledger(&self) -> LedgerAdvance {
        match self {
            Self::Save { ledger, .. } => *ledger,
        }
    }
}

impl std::fmt::Display for ClearConversationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Save { error, .. } => write!(f, "failed to save cleared session: {error}"),
        }
    }
}

impl std::error::Error for ClearConversationError {}

#[cfg(test)]
#[path = "clear_conversation_tests.rs"]
mod tests;
