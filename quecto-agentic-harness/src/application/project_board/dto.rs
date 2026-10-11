//! What the project board's use cases answer and refuse.
use std::fmt;

use super::ports::{BoardClockError, BoardHead, BoardStoreError, QuarantinedFile};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::schema_error::SchemaError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

/// A change to one task, made by the board's rules
/// (`domain::project_board::services`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardOp {
    /// The holder's claim runs two hours from now again.
    Renew,
    /// Take an expired claim over.
    TakeOver,
    /// Move the task through the transition table; `Move(Claimed)` claims
    /// a ready task.
    Move(TaskStatus),
    /// The holder links a pull request.
    AddPr(u64),
    /// The holder unlinks a pull request closed without merging.
    RemovePr(u64),
    /// The holder ends the task's open run as lost.
    EndRunLost,
}

impl BoardOp {
    /// The board commit message of this op on task `id`.
    pub fn message(&self, id: &Slug) -> String {
        let id = id.as_str();
        match self {
            Self::Renew => format!("board: renew {id}"),
            Self::TakeOver => format!("board: take over {id}"),
            Self::Move(to) => format!("board: move {id} to {}", to.as_str()),
            Self::AddPr(pr) => format!("board: link pull request {pr} to {id}"),
            Self::RemovePr(pr) => format!("board: unlink pull request {pr} from {id}"),
            Self::EndRunLost => format!("board: end {id}'s run as lost"),
        }
    }
}

/// Tasks a query found, by id, and every file the board set aside, so a
/// listing never hides that part of the board could not be read; both as
/// the board stood at `head`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardListing {
    /// The head the listing was read at.
    pub head: BoardHead,
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
    /// The task's file is set aside as unreadable: mend it first.
    Quarantined(Slug),
    /// A new task must be an unclaimed draft or ready task with no pull
    /// requests, runs or reviews yet.
    NotCreatable(String),
    /// A new task names a parent or dependency the board does not hold.
    UnknownReference(Slug),
    /// The task was stamped past the reference by a clock that ran ahead:
    /// it is not built on until real time catches up.
    TaskAhead {
        id: Slug,
        updated: Timestamp,
    },
    /// The outside reference reads a time the board cannot spell.
    ClockOutOfRange(Timestamp),
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
            Self::Store(BoardStoreError::Clash(why)) => {
                write!(f, "the board clashes with the repository: {why}")
            }
            Self::Store(BoardStoreError::InvalidWrite(why)) => {
                write!(f, "the board refused an invalid write: {why}")
            }
            Self::Store(BoardStoreError::Unavailable(why)) => {
                write!(f, "the board store is unavailable: {why}")
            }
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
            Self::Quarantined(id) => write!(
                f,
                "task {} is set aside as unreadable; mend its file",
                id.as_str()
            ),
            Self::NotCreatable(why) => write!(f, "a new task {why}"),
            Self::UnknownReference(id) => {
                write!(f, "no task {} on the board to refer to", id.as_str())
            }
            Self::TaskAhead { id, updated } => write!(
                f,
                "task {} was stamped {} by a clock that ran ahead",
                id.as_str(),
                updated.as_str()
            ),
            Self::ClockOutOfRange(reference) => write!(
                f,
                "the reference time {} is out of range",
                reference.as_str()
            ),
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
