//! The board service's contracts (epic #2705 B4): what a board server
//! answers of its board, and the port that names the files a board is.
//! The server reads its board only through [`ProjectBoardStore`], so it
//! serves the git store today and a SQLite store later unchanged.
//!
//! [`ProjectBoardStore`]: super::ProjectBoardStore
use super::{BoardHead, BoardSnapshot, QuarantinedFile};
use crate::application::project_board::dto::ProjectBoardError;
use crate::domain::project_board::value_objects::telemetry::BoardId;

/// One board file: its path from the board's root (`tasks/<id>.md`,
/// `plans/<id>.<n>.md`, ...) and its bytes, as the board's store keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// Port: the files a snapshot is, each artifact's file where a read of
/// the board finds it, written by its kind's codec.
pub trait SnapshotFiles: Send + Sync {
    /// The files of `snapshot`'s artifacts, in path order, each path once.
    ///
    /// # Errors
    /// An artifact no file of the board can hold: what its codec said.
    fn files(&self, snapshot: &BoardSnapshot) -> Result<Vec<BoardFile>, String>;
}

/// What a board server is to its board: an allowlist. A follower (B11)
/// is added when one exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerRole {
    Primary,
}

impl ServerRole {
    /// How the wire spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
        }
    }
}

/// A board server's status: its role, the board it serves, and that
/// board's head as its store gives it (`None` while there is no board).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedStatus {
    pub role: ServerRole,
    pub board: BoardId,
    pub head: Option<BoardHead>,
}

/// The served board's files at its head, with what its store set aside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedSnapshot {
    pub board: BoardId,
    pub head: BoardHead,
    pub files: Vec<BoardFile>,
    pub set_aside: Vec<QuarantinedFile>,
}

/// Why a board server could not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeError {
    /// The request named a board this server does not serve.
    UnknownBoard,
    /// The board's store or clock failed, or there is no board.
    Board(ProjectBoardError),
    /// The board holds an artifact no file can hold.
    Files(String),
}
