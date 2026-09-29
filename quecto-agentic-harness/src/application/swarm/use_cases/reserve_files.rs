//! `Tasks.reserve(task_id, token, paths)` (#2275): the owner of a claim
//! reserves files in the shared checkout, all or none.
use std::sync::Arc;

use crate::application::swarm::dto::{Reservation, ReserveFilesRequest};
use crate::application::swarm::ports::{BoardRepository, CheckoutPaths, Clock, IdSource};
use crate::domain::swarm::BoardError;

pub struct ReserveFiles {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    ids: Arc<dyn IdSource>,
    checkout: Arc<dyn CheckoutPaths>,
}

impl ReserveFiles {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        ids: Arc<dyn IdSource>,
        checkout: Arc<dyn CheckoutPaths>,
    ) -> Self {
        Self {
            repository,
            clock,
            ids,
            checkout,
        }
    }

    pub fn execute(&self, request: ReserveFilesRequest) -> Result<Reservation, BoardError> {
        let _ = (
            &self.repository,
            &self.clock,
            &self.ids,
            &self.checkout,
            request,
        );
        Err(BoardError::new("reserve is not served yet"))
    }
}

#[cfg(test)]
#[path = "reserve_files_tests.rs"]
mod tests;
