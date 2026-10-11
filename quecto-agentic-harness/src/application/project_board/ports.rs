//! The project board capability's ports (#2476): where the board lives
//! (the S1d git store implements [`ProjectBoardStore`] over the
//! repository's `quecto/board` branch) and what time it is.
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// The board's head: the store's revision a write must start from (the
/// git store's commit id).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoardHead(pub String);

/// A board file the store set aside, and why. `id` is the task id its file
/// name spells (`tasks/<id>.json`), when it spells one: no task may be
/// written under that id until the file is mended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantinedFile {
    pub path: String,
    pub id: Option<Slug>,
    pub reason: String,
}

/// The board at its head: every readable task, and what was set aside.
/// The order of either list is the store's own; read them as sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardSnapshot {
    pub head: BoardHead,
    pub tasks: Vec<Task>,
    pub quarantined: Vec<QuarantinedFile>,
}

/// What became of a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardWrite {
    /// The board moved to this head, and it holds the write.
    Written(BoardHead),
    /// The board does NOT hold this write: someone else moved it first.
    /// Read it again. A store that cannot tell whether its write landed
    /// answers neither this nor [`Self::Written`] but an error.
    Lost,
    /// The store refused the write, and the board did not move.
    Refused(String),
}

/// What [`ProjectBoardStore::bootstrap`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardBootstrap {
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
    /// A write no store takes: no tasks, one id twice, a message that is
    /// not one short line, a board past its bounds, or a head that is not
    /// on the board. That includes a head the board held before its
    /// history was rewritten (a force push): such a write is invalid, not
    /// [`BoardWrite::Lost`], since no store can tell it from a head it
    /// never held. Read the board again. Nothing was written.
    InvalidWrite(String),
    /// The store could not be reached or read, or cannot tell whether a
    /// write landed.
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
    /// one change, only if the board is still at `expected`. A task whose
    /// id a quarantined file holds is [`BoardWrite::Refused`].
    ///
    /// # Errors
    /// An invalid write, or the store's failure; losing the race is
    /// [`BoardWrite::Lost`].
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
    fn bootstrap(&self) -> Result<BoardBootstrap, BoardStoreError>;

    /// The committer identity the store writes as: every board change is
    /// made by it. It cannot fail: a store resolves its identity, and
    /// checks it is a valid one ([`Identity::validate`]), when it is built,
    /// and refuses to be built without one.
    fn writer(&self) -> Identity;
}

/// Why the clock could not tell the time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardClockError {
    /// This machine's clock reads a time the board cannot spell (before
    /// 1970, or past 9999).
    OutOfRange(String),
}

/// What time it is, an outside reading of it to check that against, and
/// the pause between attempts at a write. Blocking: [`Self::pause`] holds
/// the calling thread, so callers run board changes off the async workers,
/// as they do the store's.
pub trait BoardClock: Send + Sync {
    /// This machine's time.
    ///
    /// # Errors
    /// A time the board cannot spell.
    fn now(&self) -> Result<Timestamp, BoardClockError>;

    /// The time a source outside this machine reports, when one can be
    /// read; `None` when none can.
    fn reference(&self) -> Option<Timestamp>;

    /// Waits before attempt `attempt` (from 2) at a write that lost the
    /// race: a short, jittered pause, so writers that collided spread out.
    /// It blocks the calling thread for the pause.
    fn pause(&self, attempt: usize);
}

/// Where a pull request stands, as the repository's host reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullRequestState {
    Open,
    Merged,
    /// Closed without merging.
    Closed,
    /// The host does not know this pull request.
    Unknown,
}

/// Why the host could not tell where a pull request stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullRequestStatesError {
    Unavailable(String),
}

/// The repository host's reading of the pull requests a task links.
/// Blocking: callers run it off the async workers.
pub trait PullRequestStates: Send + Sync {
    /// Where pull request `pr` stands now.
    ///
    /// # Errors
    /// The host could not be reached or read.
    fn state(&self, pr: u64) -> Result<PullRequestState, PullRequestStatesError>;
}
