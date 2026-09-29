//! `Workbench.withdraw(message_id)` (#2276, #1837): the sender takes back
//! its own unread message.
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_messages::{Retirement, message_id, retire};
use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::dto::{MessageIdRequest, Settled};
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

/// The id (an integer of at least 1) is checked before the operation gate,
/// which any member passes while the run is not over (bookkeeping, so a
/// paused run withdraws too). The caller's own message moves from
/// `accepted` to `withdrawn`, leaving the recipient's inbox but not the
/// audit, and the event `message_withdrawn{message}` records it; a message
/// already withdrawn is left as it is, with no event.
pub struct WithdrawMessage {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    encoding: Arc<dyn BoardEncoding>,
}

impl WithdrawMessage {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        encoding: Arc<dyn BoardEncoding>,
    ) -> Self {
        Self {
            repository,
            clock,
            encoding,
        }
    }

    /// # Errors
    /// `message id must be a positive integer`, an authorisation refusal,
    /// a message that is not the caller's or not unread, or the store's.
    pub fn execute(&self, request: MessageIdRequest) -> Result<Settled, BoardError> {
        let id = message_id(&request.message_id)?;
        let actor = request.actor.as_str();
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            Access::default(),
            |transaction, _| {
                let changed = retire(
                    transaction,
                    &*self.encoding,
                    actor,
                    id,
                    None,
                    Retirement::Withdrawn,
                )?;
                if changed {
                    transaction.event(
                        actor,
                        self.clock.now_seconds(),
                        "message_withdrawn",
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

impl OverRepository for WithdrawMessage {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
            encoding: self.encoding.clone(),
        }
    }
}

#[cfg(test)]
#[path = "withdraw_message_tests.rs"]
mod tests;
