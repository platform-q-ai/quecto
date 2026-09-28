//! The board ports over the SQLite store (#2270): red-phase skeleton.
use super::store::BoardStore;
use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::{BoardRepository, BoardWork};
use crate::domain::swarm::BoardError;

/// The board file of one run.
#[derive(Clone, Debug)]
pub struct SqliteBoardRepository {
    store: BoardStore,
}

impl SqliteBoardRepository {
    pub fn new(location: &BoardLocation) -> Self {
        Self {
            store: BoardStore::new(&location.database),
        }
    }
}

impl BoardRepository for SqliteBoardRepository {
    fn atomic(&self, create: bool, work: &mut BoardWork<'_>) -> Result<(), BoardError> {
        let _ = (&self.store, create, work);
        Err(BoardError::new("not implemented yet (#2270)"))
    }
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod tests;
