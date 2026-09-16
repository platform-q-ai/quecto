//! Boundary values of retained context (#1866, D9 #1978): what a caller
//! asks the recall use case for, what it gets back, and the receipt the
//! retention writer returns. Domain values only — the entry vocabulary is
//! the domain's (`SpillEntry`, `SpillIndex`, `SpillId`); no wire value, no
//! tool schema, no path.

use crate::domain::error::DomainError;
use crate::domain::session::{SpillEntries, SpillEntry};
use crate::domain::session_identity::SpillId;

/// The reserved id that selects the retained-context index instead of one
/// entry: `recall("list")`. Part of the recall syntax the model sees.
pub const INDEX_QUERY: &str = "list";

/// What a recall request selects: the whole index of the session, or one
/// retained entry by its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecallQuery {
    /// The session's retained-context index (`recall("list")`).
    Index,
    /// One retained entry, by the id a collapse stub or the index showed.
    Entry(SpillId),
}

impl RecallQuery {
    /// Admit the id a caller supplied: the reserved [`INDEX_QUERY`] selects
    /// the index, any other non-empty id selects that entry, and an empty
    /// id (a malformed or absent argument) is refused as
    /// [`RecallError::MalformedId`] — nothing in the store is ever named by
    /// the empty id, so no lookup is made for it.
    pub fn parse(id: &str) -> Result<Self, RecallError> {
        if id.is_empty() {
            return Err(RecallError::MalformedId);
        }
        if id == INDEX_QUERY {
            return Ok(Self::Index);
        }
        Ok(Self::Entry(SpillId::new(id)))
    }
}

/// What a recall produced.
#[derive(Debug, Clone)]
pub enum RecallOutcome {
    /// The index of the session's retained entries, oldest first (append
    /// order), without content.
    Index(SpillEntries),
    /// The one entry the id named, with its content.
    Entry(SpillEntry),
    /// No retained entry of this session carries the id.
    Missing(SpillId),
}

/// Why a recall did not produce an outcome.
#[derive(Debug)]
pub enum RecallError {
    /// The id was empty: a malformed or absent argument.
    MalformedId,
    /// The retention store failed.
    Store(DomainError),
}

impl std::fmt::Display for RecallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedId => write!(f, "recall requires a non-empty id"),
            Self::Store(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for RecallError {}

impl From<DomainError> for RecallError {
    fn from(error: DomainError) -> Self {
        Self::Store(error)
    }
}

/// The receipt of a successful append: the id the entry was retained
/// under. Equal to the caller's id, or — for a deduplicated append — the
/// caller's id with the `:{k}` suffix that made it unique. Issued only
/// after the append succeeded, so a caller that stamps it on a message
/// never mints a stub the store cannot resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retained {
    pub id: String,
}

#[cfg(test)]
#[path = "retained_context_tests.rs"]
mod tests;
