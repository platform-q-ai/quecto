//! Building the fence's tree (epic #2705 B13) from a board commit this
//! clone holds, and publishing it through the store's existing exact-lease
//! push and reconciliation. The pure tree builder remains the sole owner.
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

use crate::application::project_board::ports::fence::{BoardFence, FenceError, FenceReport};

/// Offline composition supplies a lazy caller writer: exact Already never
/// asks for fresh identity, and the placeholder reader never creates commits.
pub struct GitBoardFence {
    store: GitBoardStore,
    writer: Box<dyn Fn() -> Result<Git, BoardStoreError> + Send + Sync>,
}
impl GitBoardFence {
    pub fn new(
        store: GitBoardStore,
        writer: impl Fn() -> Result<Git, BoardStoreError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            store,
            writer: Box::new(writer),
        }
    }
}
impl BoardFence for GitBoardFence {
    fn publish_fence(
        &self,
        captured: &BoardHead,
        moved: &BoardMoved,
    ) -> Result<FenceReport, FenceError> {
        self.store
            .publish_fence_as(captured, moved, || (self.writer)())
    }
}
impl BoardFence for GitBoardStore {
    fn publish_fence(
        &self,
        captured: &BoardHead,
        moved: &BoardMoved,
    ) -> Result<FenceReport, FenceError> {
        self.publish_fence_as(captured, moved, || Ok(self.git.clone()))
    }
}

impl GitBoardStore {
    fn publish_fence_as(
        &self,
        captured: &BoardHead,
        moved: &BoardMoved,
        writer: impl FnOnce() -> Result<Git, BoardStoreError>,
    ) -> Result<FenceReport, FenceError> {
        object_id(&captured.0).map_err(FenceError::Store)?;
        self.head()
            .map_err(FenceError::Store)?
            .ok_or(FenceError::Store(BoardStoreError::NoBoard))?;
        let current = self.fetch().map_err(FenceError::Store)?;
        if current == captured.0 {
        } else {
            return Err(FenceError::Conflict);
        }
        let tree = match self
            .fence_tree(&current, moved)
            .map_err(FenceError::Store)?
        {
            Fence::Already => {
                return Ok(FenceReport::Already {
                    head: captured.clone(),
                });
            }
            Fence::Tree(tree) => tree,
        };
        let git = writer().map_err(FenceError::Store)?;
        let commit = git
            .text(
                &[
                    "commit-tree",
                    &tree,
                    "-p",
                    &captured.0,
                    "-m",
                    "board: move to board server",
                ],
                None,
            )
            .map_err(|error| FenceError::Store(unavailable(error)))?;
        let commit = object_id(&commit).map_err(FenceError::Store)?;
        assert_ne!(
            commit, captured.0,
            "a fence commit has its captured predecessor as sole parent"
        );
        match self.push(Some(&captured.0), &commit) {
            Ok(Pushed::Landed) => Ok(FenceReport::Published {
                previous: captured.clone(),
                head: BoardHead(commit),
            }),
            Ok(Pushed::Lost) => Err(FenceError::Conflict),
            Ok(Pushed::Refused(reason)) => Err(FenceError::Refused(reason)),
            Err(error) => Err(FenceError::Uncertain(told(&error).into_owned())),
        }
    }
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

#[cfg(test)]
#[path = "store_fence_tests.rs"]
mod publication_tests;
