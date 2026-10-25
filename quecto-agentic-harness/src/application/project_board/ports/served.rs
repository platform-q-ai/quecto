//! The board service's contracts (epic #2705 B4, B7).
use std::sync::Arc;

use super::{BoardHead, BoardOperations, BoardStoreError, ProjectBoardStore};
use crate::application::project_board::dto::{BoardRequest, ProjectBoardError, RunAfterUndo};
use crate::application::project_board::run_facts::RunFacts;
use crate::application::project_board::start::{NotedFoldBack, StartUndo};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::telemetry::BoardId;

/// What a board server is to its board: an allowlist (a follower is B11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerRole {
    Primary,
}

impl ServerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
        }
    }
}

/// A board server's status; `head` is `None` while there is no board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedStatus {
    pub role: ServerRole,
    pub board: BoardId,
    pub head: Option<BoardHead>,
}

/// Why a board server could not answer (the write cases are B7's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeError {
    Board(ProjectBoardError),
    NoWrite,
    WriteIdReused,
    Busy,
    /// The remote is being read for another write (B8): busy, retried.
    Reading,
}

/// A write's id (B7), a UUID, the same on every retry of the write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteId(String);

impl WriteId {
    /// `text` as a write id: lowercase hex in groups of 8-4-4-4-12.
    pub fn parse(text: &str) -> Option<Self> {
        let shaped = text.len() == 36
            && text.char_indices().all(|(at, c)| match at {
                8 | 13 | 18 | 23 => c == '-',
                _ => matches!(c, '0'..='9' | 'a'..='f'),
            });
        shaped.then(|| Self(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One write (B7): its id, what it asks (front end and text), the identity
/// asserted (its author), and who the server let in (recorded beside it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpWrite {
    pub write_id: WriteId,
    pub request: String,
    pub committer: Identity,
    pub principal: String,
}

/// A served write's answer, made now or kept; or its id kept for another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transacted {
    Answered(String),
    Replayed(String),
    Reused,
}

/// Port: a board server's store, written one operation at a time (B7).
pub trait BoardOpStore: Send + Sync {
    /// The store a read is answered from, outside any write.
    fn reads(&self) -> Arc<dyn ProjectBoardStore>;

    /// `write`'s kept answer, read outside any write (`transact` judges).
    ///
    /// # Errors
    /// The store's failure.
    fn kept(&self, write: &OpWrite) -> Result<Option<Transacted>, BoardStoreError>;

    /// Runs `op` once per write id and principal, under the write lock in
    /// one immediate transaction, on a store writing as `write.committer`;
    /// its answer is kept as it commits, and replayed for the same id.
    ///
    /// # Errors
    /// The store's failure, or `op`'s; either writes nothing.
    fn transact(
        &self,
        write: &OpWrite,
        op: &mut dyn FnMut(Arc<dyn ProjectBoardStore>) -> Result<String, BoardStoreError>,
    ) -> Result<Transacted, BoardStoreError>;
}

/// The operations over the store they are handed.
pub type OperationsOver = Box<
    dyn Fn(Arc<dyn ProjectBoardStore>) -> Result<Box<dyn BoardOperations>, ProjectBoardError>
        + Send,
>;

/// Port: the board's operations, as composition assembles them (B7).
pub trait BoardOperationsOver: Send + Sync {
    /// The operations for `request` and what its client sent for its
    /// `run`, its host reads made now, before any lock.
    fn prepare(
        &self,
        request: Option<&BoardRequest>,
        run: &ServedRun,
        reads: &dyn ProjectBoardStore,
    ) -> Result<OperationsOver, ServeError>;
}

/// What a client sends beside a request for its run (B8): an end's
/// facts, or the undo of a start it began and could not launch. Facts
/// are trusted cooperatively: a client that lets an end through says what
/// its own front end saw (its coordinator still running, its swarm's
/// board), as the local board trusts its own front end; the server reads
/// the remote and the host itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServedRun {
    Plain,
    Facts(RunFacts),
    Undo(StartUndo),
}

/// Where an undo left its run, its fold-back where made here, and the
/// board's refusal of it, when it refused.
pub type Undone = (RunAfterUndo, Option<NotedFoldBack>, Option<String>);

/// Port: where a begun start is undone (B8): the board it was begun on,
/// this machine's or a board server's.
pub trait BegunStarts: Send + Sync {
    /// `undo`'s run folded back as failed, as a swarm that reported
    /// nothing, moving its task back as the start's undo does: where that
    /// left the run (open when it could not be folded back), and the
    /// fold-back for its record where it was made here, when `noting`; and
    /// the board's refusal of the undo, its code and words, when it refused.
    fn undo(&self, undo: &StartUndo, noting: bool) -> Undone;

    /// The time since this board's clock was made, on a monotonic clock;
    /// `None` where no clock is read (a board server's client).
    fn elapsed(&self) -> Option<std::time::Duration>;
}
