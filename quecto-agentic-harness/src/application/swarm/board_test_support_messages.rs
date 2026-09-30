//! Test support (compiled only under `cfg(test)`): the in-memory board's
//! messages (#2275, #2276). A message id matches by the rough INTEGER
//! affinity of `board_test_support_tasks`, a recipient by its text, and
//! the inbox flag by Python's truth; the SQLite adapter's contract tests
//! pin the real binding.
use serde_json::Value;

use super::tasks::affinity;
use super::{MemoryTransaction, text_affinity};
use crate::application::swarm::dto::{MessageRow, MessageTally, NewMessage};
use crate::application::swarm::ports::BoardMessages;
use crate::domain::swarm::{BoardError, python_truthy};

/// One `messages` row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoredMessage {
    pub id: i64,
    pub sender: String,
    pub recipient: Value,
    pub body: String,
    pub status: String,
    pub revision: Option<String>,
    pub supersedes: Option<i64>,
    pub superseded_by: Option<i64>,
}

impl StoredMessage {
    /// `dict(row)`, in the migrated table's column order.
    fn row(&self) -> MessageRow {
        let columns = [
            ("id", Value::from(self.id)),
            ("sender", Value::from(self.sender.clone())),
            ("recipient", self.recipient.clone()),
            ("body", Value::from(self.body.clone())),
            ("status", Value::from(self.status.clone())),
            (
                "revision",
                self.revision.clone().map_or(Value::Null, Value::from),
            ),
            (
                "supersedes",
                self.supersedes.map_or(Value::Null, Value::from),
            ),
            (
                "superseded_by",
                self.superseded_by.map_or(Value::Null, Value::from),
            ),
        ];
        MessageRow {
            columns: columns
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        }
    }

    fn addressed_to(&self, recipient: &str) -> bool {
        recipient_text(&self.recipient).as_deref() == Some(recipient)
    }
}

/// A recipient as the TEXT column keeps it (`5` and `"5"` alike), or
/// `None` for NULL, which matches no recipient.
fn recipient_text(recipient: &Value) -> Option<String> {
    match text_affinity(recipient) {
        Value::String(text) => Some(text),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            None
        }
    }
}

impl MemoryTransaction<'_> {
    /// The id of the stored message `id` finds, when one does.
    fn found(&self, id: &Value) -> Option<usize> {
        let id = affinity(id)?;
        self.state
            .borrow()
            .messages
            .iter()
            .position(|message| message.id == id)
    }

    fn next_message_id(&self) -> i64 {
        self.state
            .borrow()
            .messages
            .last()
            .map_or(1, |message| message.id + 1)
    }
}

impl BoardMessages for MemoryTransaction<'_> {
    fn inbox_count(&self, recipient: &Value) -> Result<i64, BoardError> {
        let Some(wanted) = recipient_text(recipient) else {
            return Ok(0);
        };
        let messages = &self.state.borrow().messages;
        let count = messages
            .iter()
            .filter(|message| message.addressed_to(&wanted) && message.status == "accepted")
            .count();
        Ok(i64::try_from(count).unwrap())
    }

    fn insert_message(
        &self,
        sender: &str,
        recipient: &Value,
        body: &str,
    ) -> Result<i64, BoardError> {
        self.note(format!("insert_message {recipient}"));
        let id = self.next_message_id();
        self.state.borrow_mut().messages.push(StoredMessage {
            id,
            sender: sender.to_owned(),
            recipient: recipient.clone(),
            body: body.to_owned(),
            status: "accepted".to_owned(),
            ..StoredMessage::default()
        });
        Ok(id)
    }

    fn message(&self, id: &Value) -> Result<Option<MessageRow>, BoardError> {
        let found = self.found(id);
        Ok(found.map(|index| self.state.borrow().messages[index].row()))
    }

    fn addressed_message(
        &self,
        id: &Value,
        recipient: &str,
    ) -> Result<Option<MessageRow>, BoardError> {
        let found = self.found(id);
        let messages = &self.state.borrow().messages;
        Ok(found
            .map(|index| &messages[index])
            .filter(|message| message.addressed_to(recipient))
            .map(StoredMessage::row))
    }

    fn set_message_status(&self, id: &Value, status: &str) -> Result<(), BoardError> {
        self.note(format!("set_message_status {id} {status}"));
        let index = self.found(id).expect("a message read in this transaction");
        self.state.borrow_mut().messages[index].status = status.to_owned();
        Ok(())
    }

    fn message_tally(&self) -> Result<MessageTally, BoardError> {
        let messages = &self.state.borrow().messages;
        let count = |status: &str| {
            let counted = messages.iter().filter(|m| m.status == status).count();
            i64::try_from(counted).unwrap()
        };
        Ok(MessageTally {
            sent: i64::try_from(messages.len()).unwrap(),
            consumed: count("consumed"),
            withdrawn: count("withdrawn"),
        })
    }

    fn send_message(&self, message: &NewMessage) -> Result<i64, BoardError> {
        self.note(format!("send_message {}", message.recipient));
        let id = self.next_message_id();
        self.state.borrow_mut().messages.push(StoredMessage {
            id,
            sender: message.sender.clone(),
            recipient: message.recipient.clone(),
            body: message.body.clone(),
            status: "accepted".to_owned(),
            revision: message.revision.clone(),
            supersedes: message.supersedes.as_ref().and_then(affinity),
            superseded_by: None,
        });
        Ok(id)
    }

    fn set_superseded_by(&self, id: &Value, successor: i64) -> Result<(), BoardError> {
        self.note(format!("set_superseded_by {id} {successor}"));
        let index = self
            .found(id)
            .expect("a message retired in this transaction");
        self.state.borrow_mut().messages[index].superseded_by = Some(successor);
        Ok(())
    }

    fn inbox(
        &self,
        recipient: &str,
        include_consumed: &Value,
    ) -> Result<Vec<MessageRow>, BoardError> {
        let every = python_truthy(include_consumed);
        let mut messages = self.state.borrow().messages.clone();
        messages.sort_by_key(|message| message.id);
        Ok(messages
            .iter()
            .filter(|message| message.addressed_to(recipient))
            .filter(|message| every || message.status == "accepted")
            .take(100)
            .map(StoredMessage::row)
            .collect())
    }
}
