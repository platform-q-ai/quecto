//! The git store's revisions as a pin reads them (#2705 B9): the board's
//! commit is its revision, and an artifact's file is named by its object
//! as a commit on the board's history held it, from that commit's root and
//! the file's folder alone ([`object_in`]).
use super::super::board_tree::{artifact_path, object_id, object_in, unavailable};
use super::super::git::{SHORT_OUTPUT, failed};
use super::GitBoardStore;
use crate::application::project_board::ports::{ArtifactKey, BoardHead, BoardStoreError};

impl GitBoardStore {
    /// The object of `key`'s file as the board's commit `rev` held it, its
    /// head being the git store's revision: a rev that names no commit
    /// this clone holds on the board's history (as its last fetch left it)
    /// is none. A clone that lacks the commit's root or the file's folder
    /// (a partial clone) fetches that tree alone, never a file, and looks
    /// again.
    pub(super) fn held_at(
        &self,
        rev: &BoardHead,
        key: &ArtifactKey,
    ) -> Result<Option<String>, BoardStoreError> {
        let Ok(commit) = object_id(&rev.0) else {
            return Ok(None);
        };
        let on_board = match self.local_tip()? {
            Some(tip) => self.on_board(&commit, &tip)?,
            None => false,
        };
        if !on_board {
            return Ok(None);
        }
        let (folder, name) = artifact_path(key);
        // A tree this clone lacks (a partial clone) is fetched alone and
        // looked at again, as a read fetches what it misses.
        let before = super::fetch_state::FetchState::default();
        self.reading(&commit, &before, |_| {
            object_in(&self.git, &commit, (folder, &name), self.limits)
        })
    }

    /// Whether `commit` is a commit here and `tip` or one of its ancestors.
    pub(super) fn on_board(&self, commit: &str, tip: &str) -> Result<bool, BoardStoreError> {
        Ok(self.is_commit(commit)? && self.is_ancestor(commit, tip)?)
    }

    /// The board's commit as this clone last fetched it, if it has.
    pub(super) fn local_tip(&self) -> Result<Option<String>, BoardStoreError> {
        let commit = format!("{}^{{commit}}", self.tracking());
        let args = ["rev-parse", "--verify", "--quiet", commit.as_str()];
        let output = self
            .git
            .local(&args, None, SHORT_OUTPUT)
            .map_err(unavailable)?;
        match output.status.code() {
            Some(0) => Ok(Some(object_id(
                String::from_utf8_lossy(&output.stdout).trim_end(),
            )?)),
            Some(1) => Ok(None),
            _ => Err(unavailable(failed(&args, &output))),
        }
    }
}
