//! RED STUB (#2271).
use std::sync::Arc;

use super::{ActivateMember, AdmitMember};
use crate::application::swarm::dto::{JoinRunRequest, Joined};
use crate::application::swarm::ports::{BoardRepository, IdSource};
use crate::domain::swarm::BoardError;

pub struct JoinRun {
    repository: Arc<dyn BoardRepository>,
    ids: Arc<dyn IdSource>,
    admit: Arc<AdmitMember>,
    activate: Arc<ActivateMember>,
}

impl JoinRun {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        ids: Arc<dyn IdSource>,
        admit: Arc<AdmitMember>,
        activate: Arc<ActivateMember>,
    ) -> Self {
        Self {
            repository,
            ids,
            admit,
            activate,
        }
    }

    /// # Errors
    /// Not implemented yet.
    pub fn execute(&self, _request: JoinRunRequest) -> Result<Joined, BoardError> {
        let _stubbed = (&self.repository, &self.ids, &self.admit, &self.activate);
        Err(BoardError::new("not implemented (#2271)"))
    }
}

#[cfg(test)]
#[path = "join_run_tests.rs"]
mod tests;
