//! `Workbench._notifications(with_generation=False)` (#2276): the members
//! the caller's own board changes wake, claimed once.
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_operation::operation;
use crate::application::swarm::board_wakes::{running, woken};
use crate::application::swarm::dto::{ClaimNotificationsRequest, NotificationBatch};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError, python_truthy};

/// A read-only operation (`operation(active=False, read_only=True)`): the
/// events the caller recorded after its notification cursor are judged
/// from its own view ([`crate::domain::swarm::notification_targets`]),
/// and its cursor advances to the board's generation in the same
/// transaction, before the hints are sent, so concurrent claims never
/// duplicate a hint. Hints are best-effort, not durable delivery: a run
/// that is not running wakes nobody (its events are not read) and still
/// advances the cursor.
pub struct ClaimNotifications {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ClaimNotifications {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, the policy's `TypeError` for targets that
    /// do not sort, or the store's.
    pub fn execute(
        &self,
        request: ClaimNotificationsRequest,
    ) -> Result<NotificationBatch, BoardError> {
        let with_generation = python_truthy(&request.with_generation);
        let actor = request.actor.as_str();
        let reading = Access {
            read_only: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            reading,
            |transaction, run| {
                // Python's generator of events is consumed only by a
                // running run's policy.
                let events = if running(run) {
                    transaction.notification_events(actor)?
                } else {
                    Vec::new()
                };
                let members = woken(transaction, run, actor, &events)?;
                let cursor = transaction.advance_notifications(actor)?;
                debug_assert!(cursor.current >= 0, "an event id or 0");
                Ok(NotificationBatch {
                    members,
                    generation: cursor.current,
                    with_generation,
                    cursor_moved: cursor.moved(),
                })
            },
        )
    }
}

impl OverRepository for ClaimNotifications {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "claim_notifications_tests.rs"]
mod tests;
