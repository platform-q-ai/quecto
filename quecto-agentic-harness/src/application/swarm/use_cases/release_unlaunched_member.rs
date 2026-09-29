//! `Workbench._release_unlaunched` (#2271): a launch that never started
//! gives its admission back.
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_membership::unlaunched;
use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::dto::ReleaseUnlaunchedMemberRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError, RefusalKind};

/// Through the operation gate (`active=False`): only a `reserved` member
/// with no process recorded is released. It is marked dead (so its place
/// is free and its identity spent), with the event `launch_abandoned`.
pub struct ReleaseUnlaunchedMember {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReleaseUnlaunchedMember {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, `only an unlaunched reservation may be
    /// released`, or the store's.
    pub fn execute(&self, request: ReleaseUnlaunchedMemberRequest) -> Result<(), BoardError> {
        let member = &request.member;
        let actor = request.actor.as_str();
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            Access::default(),
            |transaction, _run| {
                let row = transaction.member_row(member, None)?;
                if !row.as_ref().is_some_and(unlaunched) {
                    return Err(BoardError::new(
                        RefusalKind::WrongState,
                        "only an unlaunched reservation may be released",
                    ));
                }
                transaction.mark_member_dead_unlaunched(member)?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "launch_abandoned",
                    &detail([("member", member.clone())]),
                )
            },
        )
    }
}

impl OverRepository for ReleaseUnlaunchedMember {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "release_unlaunched_member_tests.rs"]
mod tests;
