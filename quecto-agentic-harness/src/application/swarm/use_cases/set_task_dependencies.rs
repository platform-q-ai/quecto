//! Stub (#2272 red phase).
use std::sync::Arc;

use crate::application::swarm::dto::SetTaskDependenciesRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct SetTaskDependencies {
    repository: Arc<dyn BoardRepository>,
}

impl SetTaskDependencies {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        let _ = clock;
        Self { repository }
    }

    /// # Errors
    /// Always, until implemented.
    pub fn execute(&self, request: SetTaskDependenciesRequest) -> Result<(), BoardError> {
        let _ = (&self.repository, request);
        Err(BoardError::new("pending #2272"))
    }
}

#[cfg(test)]
#[path = "set_task_dependencies_tests.rs"]
mod tests;
