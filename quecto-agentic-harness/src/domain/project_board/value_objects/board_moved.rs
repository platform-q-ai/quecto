//! Where a board moved (epic #2705 B13): the `format/MOVED` file of a
//! board fenced at format 4, naming the board's id and the board-server
//! URLs that serve it now.
use std::fmt;

use super::telemetry::BoardId;

/// The most URLs a MOVED file names.
pub const MAX_MOVED_URLS: usize = 8;
/// The longest URL a MOVED file names.
pub const MAX_URL_BYTES: usize = 256;
/// The most bytes a MOVED file holds.
pub const MAX_MOVED_BYTES: usize = 4096;

/// A board moved to a board server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardMoved {
    board: BoardId,
    urls: Vec<String>,
}

impl BoardMoved {
    /// # Errors
    /// Why the URLs are not ones a MOVED file names.
    pub fn new(board: BoardId, urls: Vec<String>) -> Result<Self, String> {
        Ok(Self { board, urls })
    }

    /// # Errors
    /// Why `bytes` is not a MOVED file quecto writes.
    pub fn parse(_bytes: &[u8]) -> Result<Self, String> {
        Err("not parsed".into())
    }

    pub fn file(&self) -> Vec<u8> {
        vec![]
    }

    pub fn board(&self) -> &BoardId {
        &self.board
    }

    pub fn urls(&self) -> &[String] {
        &self.urls
    }
}

impl fmt::Display for BoardMoved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("moved")
    }
}

#[cfg(test)]
#[path = "board_moved_tests.rs"]
mod tests;
