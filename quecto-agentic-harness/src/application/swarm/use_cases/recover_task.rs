//! `Tasks.recover(task_id, release_files=False)` (#2275).
use std::sync::Arc;

use crate::application::swarm::dto::{RecoverTaskRequest, Recovered};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct RecoverTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl RecoverTask {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    pub fn execute(&self, request: RecoverTaskRequest) -> Result<Recovered, BoardError> {
        let _ = (&self.repository, &self.clock, request);
        Err(BoardError::new("recover is not served yet"))
    }
}

#[cfg(test)]
#[path = "recover_task_tests.rs"]
mod tests;
