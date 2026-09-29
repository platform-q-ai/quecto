//! `Workbench._activate` (#2271): a launched member is live in its process.
use std::sync::Arc;

use serde_json::Value;

use crate::application::swarm::board_membership::{
    alive, holds_reservation, launched_elsewhere, live,
};
use crate::application::swarm::board_operation::{detail, operation, text};
use crate::application::swarm::dto::ActivateMemberRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

/// Through the operation gate (`active=False`), so an expired run is ended
/// first (that transaction commits) and then refused: only a `setup` or
/// `running` run activates, only a member alive under the given
/// reservation, and a live member only in the process it already runs in.
/// The member becomes live with the process and socket, and the event
/// `activated` names it and its pid.
pub struct ActivateMember {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ActivateMember {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, `run stopped before activation`, `unknown
    /// or stale launch reservation`, `member already active in a different
    /// process`, or the store's.
    pub fn execute(&self, request: ActivateMemberRequest) -> Result<(), BoardError> {
        let member = request.member.as_str();
        let actor = request.actor.as_str();
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            Access::default(),
            |transaction, run| {
                let activating = matches!(
                    run.status.as_ref().map(|status| status.as_str()),
                    Some("setup" | "running")
                );
                if !activating {
                    return Err(BoardError::new("run stopped before activation"));
                }
                let row = transaction.member_row(member, None)?;
                // `unknown_member_status_is_not_alive` (#2295): only a live
                // or reserved member is activated.
                let Some(row) = row.filter(|row| {
                    holds_reservation(row, request.reservation.as_deref()) && alive(row)
                }) else {
                    return Err(BoardError::new("unknown or stale launch reservation"));
                };
                if live(&row) && launched_elsewhere(&row, &request.launch) {
                    return Err(BoardError::new(
                        "member already active in a different process",
                    ));
                }
                transaction.activate_member(member, &request.launch, request.socket.as_deref())?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "activated",
                    &detail([
                        ("member", text(member)),
                        ("pid", Value::from(request.launch.pid)),
                    ]),
                )
            },
        )
    }
}

#[cfg(test)]
#[path = "activate_member_tests.rs"]
mod tests;
