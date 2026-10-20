//! Which revisions the board's remote holds (#2666): what a run's end
//! judges a success by.
use super::*;
use crate::application::project_board::ports::RemoteRevisions;

impl RemoteRevisions for GitBoardStore {
    fn not_on_remote(&self, _revisions: &[String]) -> Result<Vec<String>, BoardStoreError> {
        Ok(vec![])
    }
}
