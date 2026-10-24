//! The board service's contracts (epic #2705 B4, B7). The server reads its
//! board only through `ProjectBoardStore` and `BoardClock`, and writes it
//! one operation at a time through [`BoardOpStore`], over the operations
//! [`BoardOperationsOver`] assembles.
use std::sync::Arc;

use super::{BoardHead, BoardOperations, BoardStoreError, ProjectBoardStore};
use crate::application::project_board::dto::ProjectBoardError;
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

/// Why a board server could not answer: its store's or clock's failure,
/// or a write sent without its id and committer (B7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeError {
    Board(ProjectBoardError),
    NoWrite,
}

/// A write's id (B7): the same on every retry of the write, so a server
/// answers a retry with its first answer. 8 to 64 of `0-9a-f-` (a UUID).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteId(String);

impl WriteId {
    /// `text` as a write id, when it is shaped as one.
    pub fn parse(text: &str) -> Option<Self> {
        let shaped = (8..=64).contains(&text.len())
            && text
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f' | b'-'));
        shaped.then(|| Self(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One write a server makes (B7): its id, the identity its caller asserts
/// (each revision's author), and who the server let in, recorded beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpWrite {
    pub write_id: WriteId,
    pub committer: Identity,
    pub principal: String,
}

/// A served write's answer: made now, or first given to its write id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transacted {
    Answered(String),
    Replayed(String),
}

/// Port: a board server's store, written one operation at a time (B7).
pub trait BoardOpStore: Send + Sync {
    /// The store a read is answered from, outside any write.
    fn reads(&self) -> Arc<dyn ProjectBoardStore>;

    /// Runs `op` once per write id: under the board's write lock, in one
    /// immediate transaction, on a store bound to it that writes as
    /// `write.committer`, keeping its answer as it commits. A kept id is
    /// answered with its kept answer, `op` unrun.
    ///
    /// # Errors
    /// The store's failure, or `op`'s; either writes nothing.
    fn transact(
        &self,
        write: &OpWrite,
        op: &mut dyn FnMut(Arc<dyn ProjectBoardStore>) -> Result<String, BoardStoreError>,
    ) -> Result<Transacted, BoardStoreError>;
}

/// Port: the board's operations over a store, as composition assembles
/// them for a server (B7).
pub trait BoardOperationsOver: Send + Sync {
    /// # Errors
    /// One of them could not be built.
    fn over(
        &self,
        store: Arc<dyn ProjectBoardStore>,
    ) -> Result<Box<dyn BoardOperations>, ProjectBoardError>;
}
