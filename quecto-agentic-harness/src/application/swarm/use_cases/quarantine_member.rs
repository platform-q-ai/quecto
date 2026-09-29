//! `Workbench._quarantine` (#2277, #1961): a member's harness observed
//! another member's harness gone.
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_loss::{
    LOST_HARNESS, SCOPE_UNKNOWN, end_by_loss, grace_elapsed, lost,
};
use crate::application::swarm::board_operation::{detail, operation, text};
use crate::application::swarm::dto::{Quarantine, QuarantineMemberRequest};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

/// Through the operation gate (`active=False`). A missing harness is not
/// proof that its independent execution groups stopped, so ownership is
/// kept. Observer authority (#1961): only the member's launcher records
/// its loss, and only a grace after the first authorised observation
/// (its reaper normally confirms the death first); another member
/// observes but records nothing while the launcher lives, and any member
/// may once the launcher is itself lost. A member without a launcher (the
/// created coordinator) is recorded at once. Idempotent per member: one
/// already recorded lost, or dead, is left as it is. A recorded loss is a
/// `scope_unknown` event, and ends the run by loss.
pub struct QuarantineMember {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl QuarantineMember {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, an edited loss observation, or the
    /// store's (a member it cannot bind included).
    pub fn execute(&self, request: QuarantineMemberRequest) -> Result<Quarantine, BoardError> {
        let actor = request.actor.as_str();
        let member = &request.member;
        let clock = &*self.clock;
        operation(
            &*self.repository,
            clock,
            actor,
            Access::default(),
            |transaction, run| {
                if lost(transaction, member)? {
                    return Ok(Quarantine::AlreadyLost);
                }
                // `_lost` found the member's row, so its launcher column
                // is read; a NULL launcher (or none that is text) is none.
                if let Some(launcher) = transaction.member_launcher(member)?.flatten() {
                    let authorised = actor == launcher || lost(transaction, &text(&launcher))?;
                    if !authorised {
                        return Ok(Quarantine::NotLauncher);
                    }
                    if !grace_elapsed(transaction, clock, actor, member)? {
                        return Ok(Quarantine::GracePending);
                    }
                }
                transaction.event(
                    actor,
                    clock.now_seconds(),
                    "scope_unknown",
                    &detail([("member", member.clone()), ("reason", text(SCOPE_UNKNOWN))]),
                )?;
                end_by_loss(transaction, clock, actor, run, LOST_HARNESS)?;
                Ok(Quarantine::Recorded)
            },
        )
    }
}

impl OverRepository for QuarantineMember {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "quarantine_member_tests.rs"]
mod tests;
