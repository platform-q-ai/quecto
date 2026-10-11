//! What the project board's use cases answer and refuse.
use std::fmt;

use super::ports::{
    BoardClockError, BoardHead, BoardStoreError, PullRequestStatesError, QuarantinedFile,
};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::schema_error::SchemaError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};

/// A new task: only the task's own fields. Its plan and team are plan
/// artifacts (#2481's artifact model), and its claim, runs, reviews, pull
/// requests and items are made by the board's later changes, so none can
/// be supplied here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBoardTask {
    pub id: Slug,
    pub title: String,
    pub kind: TaskKind,
    /// Markdown.
    pub description: String,
    pub status: NewBoardTaskStatus,
    pub parent: Option<Slug>,
    pub depends_on: Vec<Slug>,
}

/// An edit of a draft in place: the fields it sets (`None` keeps one as
/// it is; `parent: Some(None)` takes the parent away), and the `updated`
/// the caller read, when it read one: a draft changed since is refused as
/// stale, never edited over. Drafts are shared planning: anyone with
/// access to the board edits any draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftUpdate {
    pub id: Slug,
    pub title: Option<String>,
    /// Markdown.
    pub description: Option<String>,
    pub parent: Option<Option<Slug>>,
    pub depends_on: Option<Vec<Slug>>,
    pub expected_updated: Option<Timestamp>,
}

/// The statuses a task is created in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewBoardTaskStatus {
    Draft,
    Ready,
}

impl From<NewBoardTaskStatus> for TaskStatus {
    fn from(status: NewBoardTaskStatus) -> Self {
        match status {
            NewBoardTaskStatus::Draft => Self::Draft,
            NewBoardTaskStatus::Ready => Self::Ready,
        }
    }
}

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
    /// The holder unlinks a pull request deliberately (a wrong or
    /// abandoned one); a task whose pull requests all merged is closed by
    /// `CompleteMergedTask`, and once none is linked, Review→Done is the
    /// holder's.
    RemovePr(u64),
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
/// Why the board's remote reports no pull requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoPullRequestHost {
    /// The remote is on no host pull requests are read from.
    NotAPullRequestHost,
    /// The host is one, but cannot be asked (no `gh`, say): why.
    Unreachable(String),
}

impl fmt::Display for NoPullRequestHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAPullRequestHost => f.write_str(
                "the board remote is not on GitHub or the configured host, so no pull request \
                 states are read",
            ),
            Self::Unreachable(why) => write!(f, "pull request states cannot be read: {why}"),
        }
    }
}

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
    /// A new task names a parent or dependency the board does not hold.
    UnknownReference(Slug),
    /// Creating task `id` would close a cycle: its parent and
    /// dependencies lead, through the board, to task `by`, which already
    /// names `id` as its parent or a dependency (`id`'s own task was
    /// deleted).
    ClosesCycle {
        id: Slug,
        by: Slug,
    },
    /// The store writes as an identity no task can name.
    InvalidWriter(SchemaError),
    /// The host does not know a pull request the task links.
    PrStateUnknown(u64),
    /// The host could not tell where the task's pull requests stand.
    PullRequests(PullRequestStatesError),
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
    /// No file is set aside at this path.
    NotSetAside {
        path: String,
        object: String,
    },
    /// The board's remote reports no pull requests, so none is completed.
    NoPullRequestHost(NoPullRequestHost),
    /// Task `id` changed since the caller read it (`expected` its
    /// `updated` then, `updated` now): read it again and decide again.
    Stale {
        id: Slug,
        expected: Timestamp,
        updated: Timestamp,
    },
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
            Self::UnknownReference(id) => {
                write!(f, "no task {} on the board to refer to", id.as_str())
            }
            Self::ClosesCycle { id, by } => write!(
                f,
                "task {id} would close a cycle: it leads to task {by}, which already names {id} \
                 as its parent or a dependency",
                id = id.as_str(),
                by = by.as_str()
            ),
            Self::InvalidWriter(error) => write!(f, "the board's writer is invalid: {error}"),
            Self::PrStateUnknown(pr) => write!(f, "pull request {pr} is not known to the host"),
            Self::PullRequests(PullRequestStatesError::Unavailable(why)) => {
                write!(f, "the pull requests' states are unavailable: {why}")
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
            Self::NoPullRequestHost(why) => write!(f, "{why}"),
            Self::NotSetAside { path, object } => {
                write!(f, "no file is set aside at {path:?} as {object:?}")
            }
            Self::Stale {
                id,
                expected,
                updated,
            } => write!(
                f,
                "task {} changed since it was read (updated {}, not {}); read it again",
                id.as_str(),
                updated.as_str(),
                expected.as_str()
            ),
        }
    }
}

impl std::error::Error for ProjectBoardError {}

#[cfg(test)]
#[path = "dto_tests.rs"]
mod tests;
