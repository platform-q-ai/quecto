//! The project board capability's ports (#2476): where the board lives
//! (the S1d git store implements [`ProjectBoardStore`] over the
//! repository's `quecto/board` branch) and what time it is.
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// The board's head: the store's revision a write must start from (the
/// git store's commit id).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoardHead(pub String);

/// A board file the store set aside, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantinedFile {
    pub path: String,
    pub reason: String,
}

/// The board at its head: every readable task, and what was set aside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardSnapshot {
    pub head: BoardHead,
    pub tasks: Vec<Task>,
    pub quarantined: Vec<QuarantinedFile>,
}

/// What became of a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardWrite {
    /// The board moved to this head.
    Written(BoardHead),
    /// Someone else moved the board first: read it again.
    Lost,
    /// The store refused the write, and the board did not move.
    Refused(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bootstrapped {
    Created(BoardHead),
    Existing(BoardHead),
}

/// Why the store could not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardStoreError {
    /// No board yet: bootstrap it.
    NoBoard,
    /// Something in the repository keeps the board from existing.
    Clash(String),
    /// The store could not be reached or read.
    Unavailable(String),
}

/// Where a project's board lives. Blocking: callers run it off the async
/// workers.
pub trait ProjectBoardStore: Send + Sync {
    /// The board as it stands now.
    ///
    /// # Errors
    /// No board, or the store's failure.
    fn read(&self) -> Result<BoardSnapshot, BoardStoreError>;

    /// Writes `tasks` (each whole, replacing any task of the same id) in
    /// one change, only if the board is still at `expected`.
    ///
    /// # Errors
    /// The store's failure; losing the race is [`BoardWrite::Lost`].
    fn write(
        &self,
        expected: &BoardHead,
        tasks: &[Task],
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError>;

    /// Creates the empty board, unless it exists.
    ///
    /// # Errors
    /// A clash, or the store's failure.
    fn bootstrap(&self) -> Result<Bootstrapped, BoardStoreError>;
}

/// What time it is, and an outside reading of it to check that against.
pub trait BoardClock: Send + Sync {
    /// This machine's time.
    fn now(&self) -> Timestamp;

    /// The time a source outside this machine reports, when one can be
    /// read; `None` when none can.
    fn reference(&self) -> Option<Timestamp>;
}
