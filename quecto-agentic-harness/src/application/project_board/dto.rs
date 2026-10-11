//! What the project board's use cases answer and refuse.
use std::fmt;

use super::ports::{BoardClockError, BoardStoreError, QuarantinedFile};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::schema_error::SchemaError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// Tasks a query found, by id, and every file the board set aside, so a
/// listing never hides that part of the board could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardListing {
    pub tasks: Vec<Task>,
    pub quarantined: Vec<QuarantinedFile>,
}

/// Why a board operation did nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectBoardError {
    Store(BoardStoreError),
    /// The board's rules refused the change, read against the latest task.
    Rule(BoardRuleError),
    /// The task would break the schema.
    Invalid(SchemaError),
    NotFound(Slug),
    Exists(Slug),
    /// This machine's clock could not tell the time.
    Clock(BoardClockError),
    /// This machine's clock is more than [`CLOCK_AHEAD_SECONDS`] ahead of
    /// the outside reference.
    ///
    /// [`CLOCK_AHEAD_SECONDS`]: crate::domain::project_board::services::claims::CLOCK_AHEAD_SECONDS
    ClockAhead {
        now: Timestamp,
        reference: Timestamp,
    },
    /// Every attempt lost the race to another writer.
    Contended {
        attempts: usize,
    },
    /// The store refused the write.
    Refused(String),
}

impl From<BoardStoreError> for ProjectBoardError {
    fn from(error: BoardStoreError) -> Self {
        Self::Store(error)
    }
}

impl From<BoardClockError> for ProjectBoardError {
    fn from(error: BoardClockError) -> Self {
        Self::Clock(error)
    }
}

impl fmt::Display for ProjectBoardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(BoardStoreError::NoBoard) => f.write_str("the project has no board yet"),
            Self::Store(
                BoardStoreError::Clash(why)
                | BoardStoreError::InvalidWrite(why)
                | BoardStoreError::Unavailable(why),
            ) => f.write_str(why),
            Self::Rule(error) => write!(f, "{error}"),
            Self::Clock(BoardClockError::OutOfRange(read)) => {
                write!(
                    f,
                    "this clock reads {read}, a time the board cannot spell; fix the clock"
                )
            }
            Self::Invalid(error) => write!(f, "invalid task: {error}"),
            Self::NotFound(id) => write!(f, "no task {}", id.as_str()),
            Self::Exists(id) => write!(f, "task {} already exists", id.as_str()),
            Self::ClockAhead { now, reference } => write!(
                f,
                "this clock reads {} but the board's reference reads {}; fix the clock",
                now.as_str(),
                reference.as_str()
            ),
            Self::Contended { attempts } => {
                write!(f, "the board kept moving: {attempts} attempts lost")
            }
            Self::Refused(why) => write!(f, "the board refused the write: {why}"),
        }
    }
}

impl std::error::Error for ProjectBoardError {}

#[cfg(test)]
#[path = "dto_tests.rs"]
mod tests;
