//! The board service's contracts (epic #2705 B4): what a board server
//! answers of its board, and the port that gives the bytes a board's
//! store keeps. The server reads its board only through
//! [`ProjectBoardStore`] and [`SnapshotFiles`], so it serves the git store
//! today and a SQLite store later unchanged.
//!
//! [`ProjectBoardStore`]: super::ProjectBoardStore
use super::{BoardHead, BoardSnapshot};
use crate::application::project_board::dto::ProjectBoardError;
use crate::domain::project_board::value_objects::telemetry::BoardId;

/// One board file as the board's store keeps it, byte for byte: its path
/// from the board's root (`tasks/<id>.md`, `plans/<id>.<n>.md`, ...), its
/// bytes, and, for a file the store set aside, why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardFile {
    pub path: String,
    /// `None` only for an entry set aside that holds no file's bytes (a
    /// folder, a submodule link, or a blob the clone never fetched).
    pub bytes: Option<Vec<u8>>,
    /// Why the store set it aside; `None` for an artifact it read.
    pub set_aside: Option<String>,
}

/// Why a snapshot's files could not be given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotFilesError {
    /// Its files' bytes are past what one answer holds: why.
    TooLarge(String),
    /// The store's bytes could not be read, or are not the snapshot's: why.
    Unreadable(String),
}

/// Port: the stored files of a snapshot: one per artifact it read, at the
/// path a read finds it, and one per file it set aside, each with the
/// bytes the store keeps (never a re-encoding).
pub trait SnapshotFiles: Send + Sync {
    /// The files of `snapshot`, in path order.
    ///
    /// # Errors
    /// Files past the bound of one answer, or bytes that cannot be read.
    fn files(&self, snapshot: &BoardSnapshot) -> Result<Vec<BoardFile>, SnapshotFilesError>;
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

/// The served board's stored files at its head, those set aside included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedSnapshot {
    pub board: BoardId,
    pub head: BoardHead,
    pub files: Vec<BoardFile>,
}

/// Why a board server could not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeError {
    /// The request named a board this server does not serve.
    UnknownBoard,
    /// The board's store or clock failed, or there is no board.
    Board(ProjectBoardError),
    /// The board's files could not be given whole and true: why.
    Files(String),
    /// The board's files are past what one answer holds: why.
    TooLarge(String),
}
