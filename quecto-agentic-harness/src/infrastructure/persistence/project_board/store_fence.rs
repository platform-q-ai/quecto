//! Building the fence's tree (epic #2705 B13) from a board commit this
//! clone holds: no commit is made and nothing is pushed here; the command
//! that lands the fence is a later change's.
use super::*;
use crate::domain::project_board::value_objects::board_moved::BoardMoved;

/// What fencing a board commit makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fence {
    /// The commit is fenced already, naming the same board and URLs:
    /// nothing to do.
    Already,
    /// The fence's tree, by its id, to commit on the board's head.
    Tree(String),
}

impl GitBoardStore {
    /// The tree that fences board commit `commit` to `moved`.
    ///
    /// # Errors
    /// The commit is no board this quecto reads, it is fenced to another
    /// place ([`BoardStoreError::Moved`] naming it), or git fails.
    pub fn fence_tree(&self, _commit: &str, _moved: &BoardMoved) -> Result<Fence, BoardStoreError> {
        Err(BoardStoreError::Unavailable("no fence yet".into()))
    }
}
