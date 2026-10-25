//! Building the fence's tree (epic #2705 B13) from a board commit this
//! clone holds: no commit is made and nothing is pushed here; the command
//! that lands the fence is a later change's.
use super::super::board_tree::fence::{fence_files, fenced_root, root_entries};
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
    /// The tree that fences board commit `commit` to `moved`: the board's
    /// root, its `format` tree holding the format-4 marker and `MOVED`
    /// (`board_tree::fence`). Only a board this quecto reads is fenced; one
    /// fenced to `moved` already is [`Fence::Already`].
    ///
    /// # Errors
    /// The commit is no board this quecto reads, it is fenced to another
    /// place ([`BoardStoreError::Moved`] naming it), or git fails.
    pub fn fence_tree(&self, commit: &str, moved: &BoardMoved) -> Result<Fence, BoardStoreError> {
        match self.board_at(commit) {
            Ok(_) => {}
            Err(BoardStoreError::Moved(found)) if found == *moved => return Ok(Fence::Already),
            Err(error) => return Err(error),
        }
        let mut format = BTreeMap::new();
        for (name, bytes) in fence_files(moved) {
            let blob = self.blob(&bytes)?;
            format.insert(name.to_string(), format!("{REGULAR_FILE} blob {blob}"));
        }
        let format = self.tree(&format)?;
        let root = root_entries(&self.git, commit, self.limits)?;
        let readme = self.blob(README)?;
        let tree = self.tree(&fenced_root(&root, &readme, &format))?;
        Ok(Fence::Tree(tree))
    }
}

impl crate::application::project_board::ports::fence::BoardFence for GitBoardStore {
    fn publish_fence(
        &self,
        _captured: &BoardHead,
        _moved: &BoardMoved,
    ) -> Result<
        crate::application::project_board::ports::fence::FenceReport,
        crate::application::project_board::ports::fence::FenceError,
    > {
        Err(
            crate::application::project_board::ports::fence::FenceError::Unavailable(
                "B13-lite not implemented".into(),
            ),
        )
    }
}
