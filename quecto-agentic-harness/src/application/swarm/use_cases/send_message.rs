//! `Workbench.send(request, recipient, body, revision=None,
//! supersedes=None)` (#2276, #1837): a durable message, sent once per
//! request id.
use std::sync::Arc;

use serde_json::{Map, Value};

use super::OverRepository;
use crate::application::swarm::board_messages::{
    ACCEPTED, INBOX_CAPACITY, Retirement, is_message_id, retire,
};
use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::dto::{NewMessage, SendMessageRequest, SentMessage};
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, BoardTransaction, Clock};
use crate::domain::swarm::{Access, BoardError, RefusalKind, bounded, status_is_alive};

/// The longest message body, in UTF-8 bytes.
pub const BODY_MAX_BYTES: usize = 8192;
/// The longest message revision, in UTF-8 bytes.
pub const REVISION_MAX_BYTES: usize = 256;
/// The longest request id, in UTF-8 bytes (`Store.retry`).
const REQUEST_ID_MAX_BYTES: usize = 128;

/// The body, a revision other than null and `supersedes` other than null
/// (an integer of at least 1) are checked before the operation gate (a
/// running run). Inside it the request id is bounded and the ledger
/// replays a request seen before, keyed by `['send', recipient, body]`
/// for a plain send (the pre-#1837 shape, so an older build's keys still
/// replay) and by `+ [revision, supersedes]` otherwise. A new send needs a
/// recipient that is a live or reserved member; it supersedes the
/// caller's own unread message to that recipient first, then is refused
/// when the recipient's inbox holds a hundred unread messages, and
/// inserts the message `accepted`. The events `message_superseded
/// {message, superseded_by}` (when superseding) and `message_accepted
/// {message, recipient, revision}` follow, each at its own clock reading,
/// and `{id, status}` is the receipt the ledger stores.
pub struct SendMessage {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    encoding: Arc<dyn BoardEncoding>,
}

impl SendMessage {
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
    /// An argument refusal, an authorisation or budget refusal, `request id
    /// reused with different payload`, the ledger's capacity, an unknown
    /// or dead recipient, a message that cannot be superseded, a full
    /// inbox, or the store's.
    pub fn execute(&self, request: SendMessageRequest) -> Result<SentMessage, BoardError> {
        let body = bounded(&request.body, "message", BODY_MAX_BYTES)?;
        let revision = match &request.revision {
            Value::Null => None,
            revision => Some(bounded(revision, "message revision", REVISION_MAX_BYTES)?),
        };
        let supersedes = match &request.supersedes {
            Value::Null => None,
            supersedes if is_message_id(supersedes) => Some(supersedes),
            _ => {
                return Err(BoardError::new(
                    RefusalKind::Invalid,
                    "supersedes must be a message id",
                ));
            }
        };
        let mut payload = vec![
            Value::from("send"),
            request.recipient.clone(),
            Value::from(body),
        ];
        if revision.is_some() || supersedes.is_some() {
            payload.extend([
                revision.map_or(Value::Null, Value::from),
                supersedes.cloned().unwrap_or(Value::Null),
            ]);
        }
        let payload = Value::Array(payload);
        let actor = request.actor.as_str();
        let running = Access {
            active: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            running,
            |transaction, _| {
                let id = bounded(&request.request, "request id", REQUEST_ID_MAX_BYTES)?;
                let mut sent = false;
                let receipt = transaction.retry(actor, id, &payload, &mut || {
                    sent = true;
                    let message = NewMessage {
                        sender: actor.to_owned(),
                        recipient: request.recipient.clone(),
                        body: body.to_owned(),
                        revision: revision.map(str::to_owned),
                        supersedes: supersedes.cloned(),
                    };
                    self.send(transaction, &message)
                })?;
                Ok(SentMessage { receipt, sent })
            },
        )
    }

    /// The ledger's action: the receipt `{id, status}`.
    fn send(
        &self,
        transaction: &dyn BoardTransaction,
        message: &NewMessage,
    ) -> Result<Value, BoardError> {
        let actor = message.sender.as_str();
        let reachable = transaction
            .member_status(&message.recipient)?
            .is_some_and(|row| status_is_alive(row.status.as_deref()));
        if !reachable {
            return Err(BoardError::new(
                RefusalKind::NotFound,
                "unknown or out-of-swarm recipient",
            ));
        }
        if let Some(supersedes) = &message.supersedes {
            let retired = retire(
                transaction,
                &*self.encoding,
                actor,
                supersedes,
                Some(&message.recipient),
                Retirement::Superseded,
            )?;
            debug_assert!(retired, "superseding always retires an accepted message");
        }
        if transaction.inbox_count(&message.recipient)? >= INBOX_CAPACITY {
            debug_assert_eq!(INBOX_CAPACITY, 100, "the refusal names the capacity");
            return Err(BoardError::new(
                RefusalKind::CapacityFull,
                "recipient inbox full (100 unconsumed messages)",
            ));
        }
        let id = transaction.send_message(message)?;
        debug_assert!(id > 0, "a message id is a positive rowid");
        if let Some(supersedes) = &message.supersedes {
            transaction.set_superseded_by(supersedes, id)?;
            transaction.event(
                actor,
                self.clock.now_seconds(),
                "message_superseded",
                &detail([
                    ("message", supersedes.clone()),
                    ("superseded_by", Value::from(id)),
                ]),
            )?;
        }
        transaction.event(
            actor,
            self.clock.now_seconds(),
            "message_accepted",
            &detail([
                ("message", Value::from(id)),
                ("recipient", message.recipient.clone()),
                (
                    "revision",
                    message.revision.clone().map_or(Value::Null, Value::from),
                ),
            ]),
        )?;
        let mut receipt = Map::new();
        receipt.insert("id".to_owned(), Value::from(id));
        receipt.insert("status".to_owned(), Value::from(ACCEPTED));
        Ok(Value::Object(receipt))
    }
}

impl OverRepository for SendMessage {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
            encoding: self.encoding.clone(),
        }
    }
}

#[cfg(test)]
#[path = "send_message_tests.rs"]
mod tests;
