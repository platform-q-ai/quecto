//! The wake-notification methods of the board dispatch (#2276): the
//! harness's `_notifications` (the hints a member's own board changes
//! send) and `_accept_wake` (the receiver's check of a hint), with their
//! Python signatures and their serving. Every argument reaches the use
//! case as the JSON value passed: Python takes `with_generation` by its
//! truth and type-checks `generation` itself. A record names no task or
//! message; it says whether the op moved the caller's cursor (the
//! notification cursor, or the wake cursor), and its decision whether
//! anyone was woken. Never a member name beyond the caller's own.
use serde_json::{Map, Value};

use super::{Parameter, Served, member_row, take};
use crate::application::swarm::dto::{AcceptWakeRequest, ClaimNotificationsRequest};
use crate::application::swarm::use_cases::{AcceptWake, ClaimNotifications};
use crate::domain::swarm::{BoardError, BoardOpDetail};

/// `_notifications(with_generation=False)`.
pub(super) const NOTIFICATIONS: [Parameter; 1] = [Parameter {
    name: "with_generation",
    default: Some(|| Value::Bool(false)),
}];
/// `_accept_wake(generation)`.
pub(super) const ACCEPT_WAKE: [Parameter; 1] = [super::required("generation")];

/// `value` with `decision`, recording whether the cursor moved.
fn on_cursor(value: Value, decision: &'static str, cursor_moved: bool) -> Served {
    Served {
        value,
        decision,
        task_id: None,
        message_id: None,
        cursor_moved: Some(cursor_moved),
        detail: BoardOpDetail::NONE,
        refused: None,
        controls_run: false,
    }
}

/// The members woken, each as `dict(row)` sorted by id, or with a true
/// `with_generation` `{members, generation}`.
pub(super) fn notifications(
    claim_notifications: &ClaimNotifications,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [with_generation] = take(arguments)?;
    let batch = claim_notifications.execute(ClaimNotificationsRequest {
        actor: actor.to_owned(),
        with_generation,
    })?;
    let decision = if batch.members.is_empty() {
        "quiet"
    } else {
        "hinted"
    };
    let members = Value::Array(batch.members.into_iter().map(member_row).collect());
    let value = if batch.with_generation {
        let mut answer = Map::new();
        answer.insert("members".to_owned(), members);
        answer.insert("generation".to_owned(), Value::from(batch.generation));
        Value::Object(answer)
    } else {
        members
    };
    Ok(on_cursor(value, decision, batch.cursor_moved))
}

/// Whether the claimed events still wake the caller.
pub(super) fn accept_wake(
    accept_wake: &AcceptWake,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [generation] = take(arguments)?;
    let accepted = accept_wake.execute(AcceptWakeRequest {
        actor: actor.to_owned(),
        generation,
    })?;
    let decision = if accepted.woken { "woken" } else { "not_woken" };
    Ok(on_cursor(
        Value::Bool(accepted.woken),
        decision,
        accepted.cursor_moved,
    ))
}
