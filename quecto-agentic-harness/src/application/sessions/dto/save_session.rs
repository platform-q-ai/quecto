//! Boundary values of Save current session (#1860, D5 #1972): what a
//! caller asks for, what the transaction chose and did, and how it failed.
use crate::domain::error::DomainError;
use crate::domain::session::SubagentRestoreReason;

/// Why a save is requested; it decides whether a clean delta is allowed
/// and how the killing-exit state moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveTrigger {
    /// A routine save (after a turn, around a transition): a clean delta
    /// when nothing forces a full rewrite.
    Routine,
    /// An explicit `persist_session` request carrying the client's restore
    /// reason: always a full save; an ordinary-exit reason arms the
    /// killing-exit state, any other reason cancels it.
    Explicit {
        restore_reason: SubagentRestoreReason,
    },
    /// The final save of a loop that is leaving: always a full save.
    OrdinaryExit,
}

impl SaveTrigger {
    /// Whether this trigger forces a full save regardless of the latch.
    pub fn forces_full_save(self) -> bool {
        !matches!(self, Self::Routine)
    }
}

/// How the transaction wrote the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveMode {
    /// The whole conversation, workflow and roster were written.
    Full,
    /// Only the messages past the persisted watermark were appended; the
    /// durable prefix was trusted unchanged.
    CleanDelta,
}

/// What a save did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The session is ephemeral or unnamed: affirmatively nothing to save.
    Ephemeral,
    /// The session was written; `persisted` is the new watermark.
    Saved { mode: SaveMode, persisted: usize },
}

/// Why a save failed. The store's own error is carried verbatim so the
/// presented text is what the store reported.
#[derive(Debug)]
pub enum SaveSessionError {
    Store(DomainError),
}

impl std::fmt::Display for SaveSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for SaveSessionError {}

#[cfg(test)]
#[path = "save_session_tests.rs"]
mod tests;
