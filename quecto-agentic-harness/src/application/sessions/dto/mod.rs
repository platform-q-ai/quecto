//! Boundary DTOs of the sessions capability (#1970, #1971). Scope-neutral:
//! they name sessions by [`SessionIdentity`] and never by a filename or
//! path, and carry domain values (messages, ids, ranges) — never a wire
//! value or event.

use crate::domain::session_identity::SessionKeyPrefix;

pub mod history;
pub mod message_recovery;

pub use history::{HistoryError, HistoryPage, HistoryQuery};
pub use message_recovery::{
    ContentSelector, RecoveredContent, RecoveryError, RecoveryRequest, Utf8Range,
};

/// Which saved sessions a list query covers (#1861).
///
/// Exactly the two selections the store ever offered: every session, or the
/// sessions whose identity starts with an existing key prefix (`chat-`,
/// `cli:`). The adapter may use the prefix to skip non-matching files before
/// reading them; the prefix itself is an identity prefix, never a filename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionListQuery {
    All,
    ExistingKeyPrefix(SessionKeyPrefix),
}

impl SessionListQuery {
    /// The identity prefix this query narrows to, if any.
    pub fn key_prefix(&self) -> Option<&SessionKeyPrefix> {
        match self {
            Self::All => None,
            Self::ExistingKeyPrefix(prefix) => Some(prefix),
        }
    }
}

#[cfg(test)]
#[path = "dto_tests.rs"]
mod tests;
