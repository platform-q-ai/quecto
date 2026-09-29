//! `Workbench._join(reservation, pid, started, socket)` (#2277): the
//! join, answered with the coordinator's summary.
use std::sync::Arc;

use super::OverRepository;
use super::join_run::join;
use crate::application::swarm::board_read_models::summary;
use crate::application::swarm::dto::{JoinRunRequest, JoinedSummary};
use crate::application::swarm::ports::{BoardRepository, Clock, IdSource};
use crate::domain::swarm::BoardError;

/// `join_process`, as `JoinRun` runs it, then `coordinator.summary()`:
/// every branch ends with the summary read as the coordinator the join
/// read, the already-live branch (which writes nothing) included, so its
/// gate can still refuse (a coordinator that is nobody, or without a row)
/// and an expired run is ended first (#2310).
pub struct JoinMember {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    ids: Arc<dyn IdSource>,
}

impl JoinMember {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        ids: Arc<dyn IdSource>,
    ) -> Self {
        Self {
            repository,
            clock,
            ids,
        }
    }

    /// # Errors
    /// The join's refusal, the summary's, or the store's.
    pub fn execute(&self, request: JoinRunRequest) -> Result<JoinedSummary, BoardError> {
        let (joined, coordinator) = join(&*self.repository, &*self.clock, &*self.ids, request)?;
        let summary = summary(
            &*self.repository,
            &*self.clock,
            coordinator.as_deref(),
            None,
        )?;
        Ok(JoinedSummary { joined, summary })
    }
}

impl OverRepository for JoinMember {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
            ids: self.ids.clone(),
        }
    }
}

#[cfg(test)]
#[path = "join_member_tests.rs"]
mod tests;
