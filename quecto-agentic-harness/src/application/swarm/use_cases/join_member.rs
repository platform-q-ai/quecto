//! `Workbench._join(reservation, pid, started, socket)` (#2277): the
//! join, answered with the coordinator's summary.
use std::sync::Arc;

use super::OverRepository;
use super::join_run::{JoinRefused, join};
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
    /// The join's refusal, or the summary's after an already-live join,
    /// or the store's: each before anything was written. A refusal after
    /// the join's writes committed is answered in the summary's place.
    pub fn execute(&self, request: JoinRunRequest) -> Result<JoinedSummary, BoardError> {
        let (joined, coordinator) = match join(&*self.repository, &*self.clock, &*self.ids, request)
        {
            Ok(joined) => joined,
            Err(JoinRefused {
                refusal,
                committed: Some(joined),
            }) => {
                return Ok(JoinedSummary {
                    joined,
                    summary: Err(refusal),
                });
            }
            Err(JoinRefused {
                refusal,
                committed: None,
            }) => return Err(refusal),
        };
        let summary = summary(
            &*self.repository,
            &*self.clock,
            coordinator.as_deref(),
            None,
        );
        match summary {
            Ok(summary) => Ok(JoinedSummary {
                joined,
                summary: Ok(summary),
            }),
            Err(refusal) if joined.wrote() => Ok(JoinedSummary {
                joined,
                summary: Err(refusal),
            }),
            Err(refusal) => Err(refusal),
        }
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
