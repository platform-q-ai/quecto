//! `Workbench.ack(message_id)` (#2276): the recipient marks a message in
//! its own inbox read.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_messages::{ACCEPTED, CONSUMED, message_id};
use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::dto::{MessageIdRequest, Settled};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError, RefusalKind};

/// The id (an integer of at least 1) is checked before the operation gate,
/// which any member passes while the run is not over (bookkeeping, so a
/// paused run acknowledges too). The message must be addressed to the
/// caller; an unread one becomes `consumed`, recorded as
/// `message_consumed{message}`, and any other (consumed, superseded or
/// withdrawn) is left as it is, with no event.
pub struct AcknowledgeMessage {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl AcknowledgeMessage {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// `message id must be a positive integer`, an authorisation refusal,
    /// `unknown message in own inbox`, or the store's.
    pub fn execute(&self, request: MessageIdRequest) -> Result<Settled, BoardError> {
        let id = message_id(&request.message_id)?;
        let actor = request.actor.as_str();
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            Access::default(),
            |transaction, _| {
                let Some(row) = transaction.addressed_message(id, actor)? else {
                    return Err(BoardError::new(
                        RefusalKind::NotFound,
                        "unknown message in own inbox",
                    ));
                };
                let changed = row.get("status").and_then(Value::as_str) == Some(ACCEPTED);
                if changed {
                    transaction.set_message_status(id, CONSUMED)?;
                    transaction.event(
                        actor,
                        self.clock.now_seconds(),
                        "message_consumed",
                        &detail([("message", id.clone())]),
                    )?;
                }
                Ok(Settled {
                    message_id: id.clone(),
                    changed,
                })
            },
        )
    }
}

impl OverRepository for AcknowledgeMessage {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "acknowledge_message_tests.rs"]
mod tests;
