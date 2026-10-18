//! The durable message methods of the board dispatch (#2276): `send`,
//! `withdraw`, `inbox` and `ack`, with their Python signatures and their
//! serving. Every argument reaches the use case as the JSON value passed:
//! Python binds a recipient and the inbox flag untyped and type-checks the
//! rest at run time, so no type is refused here. A record names the
//! message by its id (#2303): the receipt's for `send` (a replay's too),
//! the argument's for `withdraw` and `ack`, which is the row's id once the
//! board found it; `inbox` acts on no one message. No op moves a message
//! cursor. Where Python answered `None`, `withdraw` and `ack` answer
//! `{message_id, changed}` (#2394): the message and whether the call
//! changed its status or found it already settled.
use serde_json::Value;

use super::{Parameter, Served, object, required, take};
use crate::application::swarm::dto::{MessageIdRequest, ReadInboxRequest, SendMessageRequest};
use crate::application::swarm::use_cases::{
    AcknowledgeMessage, ReadInbox, SendMessage, WithdrawMessage,
};
use crate::domain::swarm::{BoardError, BoardOpDetail};

/// `send(request, recipient, body, revision=None, supersedes=None)`.
pub(super) const SEND: [Parameter; 5] = [
    required("request"),
    required("recipient"),
    required("body"),
    Parameter {
        name: "revision",
        default: Some(|| Value::Null),
    },
    Parameter {
        name: "supersedes",
        default: Some(|| Value::Null),
    },
];
/// `withdraw(message_id)` and `ack(message_id)`.
pub(super) const MESSAGE_ID: [Parameter; 1] = [required("message_id")];
/// `inbox(include_consumed=False)`.
pub(super) const INBOX: [Parameter; 1] = [Parameter {
    name: "include_consumed",
    default: Some(|| Value::Bool(false)),
}];

/// `value` with `decision`, recording the message `message_id`.
fn on_message(value: Value, decision: &'static str, message_id: Option<&Value>) -> Served {
    Served {
        value,
        decision,
        task_id: None,
        message_id: message_id.and_then(Value::as_i64),
        cursor_moved: None,
        detail: BoardOpDetail::NONE,
        refused: None,
        controls_run: false,
    }
}

/// The receipt `{id, status}`, as written or as the ledger replays it.
pub(super) fn send(
    send_message: &SendMessage,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [request, recipient, body, revision, supersedes] = take(arguments)?;
    let sent = send_message.execute(SendMessageRequest {
        actor: actor.to_owned(),
        request,
        recipient,
        body,
        revision,
        supersedes,
    })?;
    let decision = if sent.sent { "sent" } else { "replayed" };
    let id = sent.receipt.get("id").cloned();
    Ok(on_message(sent.receipt, decision, id.as_ref()))
}

pub(super) fn withdraw(
    withdraw_message: &WithdrawMessage,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [message_id] = take(arguments)?;
    let settled = withdraw_message.execute(MessageIdRequest {
        actor: actor.to_owned(),
        message_id,
    })?;
    let decision = if settled.changed {
        "withdrawn"
    } else {
        "unchanged"
    };
    Ok(on_message(
        answer(&settled.message_id, settled.changed),
        decision,
        Some(&settled.message_id),
    ))
}

/// Each message to the caller as `dict(row)`.
pub(super) fn inbox(
    read_inbox: &ReadInbox,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [include_consumed] = take(arguments)?;
    let rows = read_inbox.execute(ReadInboxRequest {
        actor: actor.to_owned(),
        include_consumed,
    })?;
    let rows = rows.into_iter().map(|row| row.into_value()).collect();
    Ok(on_message(Value::Array(rows), "read", None))
}

pub(super) fn ack(
    acknowledge_message: &AcknowledgeMessage,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [message_id] = take(arguments)?;
    let settled = acknowledge_message.execute(MessageIdRequest {
        actor: actor.to_owned(),
        message_id,
    })?;
    let decision = if settled.changed {
        "consumed"
    } else {
        "unchanged"
    };
    Ok(on_message(
        answer(&settled.message_id, settled.changed),
        decision,
        Some(&settled.message_id),
    ))
}

/// `{message_id, changed}`: what `withdraw` or `ack` settled (#2394).
fn answer(message_id: &Value, changed: bool) -> Value {
    object([
        ("message_id", message_id.clone()),
        ("changed", Value::Bool(changed)),
    ])
}

#[cfg(test)]
#[path = "swarm_board_dispatch_messages_tests.rs"]
mod tests;
