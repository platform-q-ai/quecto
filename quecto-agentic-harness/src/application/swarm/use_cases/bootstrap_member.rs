//! `Workbench._bootstrap(pid, started, socket, reservation=None)`
//! (#2277): a harness's first board call in its container.
use std::sync::Arc;

use super::OverRepository;
use super::bootstrap_run::bootstrap;
use super::join_run::{JoinRefused, join};
use crate::application::swarm::board_read_models::summary;
use crate::application::swarm::dto::{
    BootstrapMemberRequest, BootstrapRunRequest, BootstrappedSummary, JoinRunRequest, Joined,
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
    /// The placeholder's refusal, or the join's or the summary's when
    /// neither the placeholder nor the join wrote anything. A refusal
    /// after either committed is answered in the summary's place, with
    /// the join's branch when it took one (#2277 final review L2).
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
        // A refusal after the placeholder or the join committed is
        // answered in the summary's place; before either, it is the call's.
        let refused = |refusal: BoardError, joined: Option<Joined>| {
            if created || joined.as_ref().is_some_and(Joined::wrote) {
                Ok(BootstrappedSummary {
                    created,
                    joined,
                    summary: Err(refusal),
                })
            } else {
                Err(refusal)
            }
        };
        let joined = join(
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
        );
        let (joined, coordinator) = match joined {
            Ok(joined) => joined,
            Err(JoinRefused { refusal, committed }) => return refused(refusal, committed),
        };
        match summary(repository, clock, coordinator.as_deref(), None) {
            Ok(summary) => Ok(BootstrappedSummary {
                created,
                joined: Some(joined),
                summary: Ok(summary),
            }),
            Err(refusal) => refused(refusal, Some(joined)),
        }
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
