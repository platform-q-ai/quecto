//! `Workbench._bootstrap(pid, started, socket, reservation=None)`
//! (#2277): a harness's first board call in its container.
use std::sync::Arc;

use super::OverRepository;
use super::bootstrap_run::bootstrap;
use super::join_run::join;
use crate::application::swarm::board_read_models::summary;
use crate::application::swarm::dto::{
    BootstrapMemberRequest, BootstrapRunRequest, BootstrappedSummary, JoinRunRequest,
    LaunchIdentity,
};
use crate::application::swarm::ports::{BoardRepository, Clock, IdSource};
use crate::domain::swarm::BoardError;

/// The placeholder when the board holds no run (`BootstrapRun`'s
/// transaction, which creates the board), then the join (`JoinRun`'s),
/// then the coordinator's summary, each committed before the next begins,
/// as Python runs them.
pub struct BootstrapMember {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    ids: Arc<dyn IdSource>,
}

impl BootstrapMember {
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
    /// The store's, the join's refusal, or the summary's.
    pub fn execute(
        &self,
        request: BootstrapMemberRequest,
    ) -> Result<BootstrappedSummary, BoardError> {
        let (repository, clock, ids) = (&*self.repository, &*self.clock, &*self.ids);
        let placeholder = BootstrapRunRequest {
            member: request.member.clone(),
            pid: request.pid.clone(),
            started: request.started.clone(),
            socket: request.socket.clone(),
        };
        let created = bootstrap(repository, clock, ids, &placeholder)?.created;
        let (joined, coordinator) = join(
            repository,
            clock,
            ids,
            JoinRunRequest {
                member: request.member,
                reservation: request.reservation,
                launch: LaunchIdentity {
                    pid: request.pid,
                    started: request.started,
                },
                socket: request.socket,
            },
        )?;
        let summary = summary(repository, clock, coordinator.as_deref(), None)?;
        Ok(BootstrappedSummary {
            created,
            joined,
            summary,
        })
    }
}

impl OverRepository for BootstrapMember {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
            ids: self.ids.clone(),
        }
    }
}

#[cfg(test)]
#[path = "bootstrap_member_tests.rs"]
mod tests;
