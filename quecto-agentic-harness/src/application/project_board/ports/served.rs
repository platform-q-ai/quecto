//! The board service's contracts (epic #2705 B4). The server reads its
//! board only through `ProjectBoardStore` and `BoardClock`, so it serves
//! the git store today and a SQLite store later unchanged.
use super::BoardHead;
use crate::application::project_board::dto::ProjectBoardError;
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

/// Why a board server could not answer: its store's or clock's failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeError {
    Board(ProjectBoardError),
}
