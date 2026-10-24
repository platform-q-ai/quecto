//! The board service's contracts (epic #2705 B4). The server reads its
//! board only through `ProjectBoardStore` and [`SnapshotFiles`], so it
//! serves the git store today and a SQLite store later unchanged.
use super::{BoardHead, BoardSnapshot};
use crate::application::project_board::dto::ProjectBoardError;
use crate::domain::project_board::value_objects::telemetry::BoardId;

/// One board file as the store keeps it, byte for byte: its path from the
/// board's root, its bytes, and, for a file set aside, why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardFile {
    pub path: String,
    /// `None` only for an entry set aside that holds no file's bytes (a
    /// folder, a submodule link, or a blob the clone never fetched).
    pub bytes: Option<Vec<u8>>,
    pub set_aside: Option<String>,
}

/// Why a snapshot's files could not be given: past one answer's bound, or
/// not readable as the snapshot's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotFilesError {
    TooLarge(String),
    Unreadable(String),
}

/// Port: a snapshot's stored files, never a re-encoding: one per artifact
/// read, at the path a read finds it, and one per file set aside.
pub trait SnapshotFiles: Send + Sync {
    /// The files of `snapshot`, in path order.
    /// # Errors
    /// Files past the bound of one answer, or bytes that cannot be read.
    fn files(&self, snapshot: &BoardSnapshot) -> Result<Vec<BoardFile>, SnapshotFilesError>;
}

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

/// The served board's stored files at its head, those set aside included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedSnapshot {
    pub board: BoardId,
    pub head: BoardHead,
    pub files: Vec<BoardFile>,
}

/// Why a board server could not answer: a board it does not serve, its
/// store's or clock's failure, files it cannot give whole and true, or
/// files past what one answer holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeError {
    UnknownBoard,
    Board(ProjectBoardError),
    Files(String),
    TooLarge(String),
}
