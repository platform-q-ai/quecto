//! The loss and death methods of the board dispatch (#2277, #1924,
//! #1961): the harness's `_quarantine`, `_confirmed_dead` and
//! `_lose_coordinator`, their Python signatures and their serving. The
//! member and the exit kind reach the use cases as the JSON values passed
//! (Python binds the member untyped and checks the kind at run time). A
//! record names no task, message or member beyond the caller; its
//! decision says what the op decided.
use serde_json::{Map, Value};

use super::{Parameter, Served, done, required, take};
use crate::application::swarm::dto::{
    ConfirmMemberDeadRequest, DeathConfirmation, LoseCoordinatorRequest, Quarantine,
    QuarantineMemberRequest,
};
use crate::application::swarm::use_cases::{ConfirmMemberDead, LoseCoordinator, QuarantineMember};
use crate::domain::swarm::BoardError;

/// `_quarantine(member)`.
pub(super) const QUARANTINE: [Parameter; 1] = [required("member")];
/// `_confirmed_dead(member, exit='orderly')`.
pub(super) const CONFIRMED_DEAD: [Parameter; 2] = [
    required("member"),
    Parameter {
        name: "exit",
        default: Some(|| Value::from("orderly")),
    },
];

/// `null`, with what the observation decided.
pub(super) fn quarantine(
    quarantine_member: &QuarantineMember,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [member] = take(arguments)?;
    let decided = quarantine_member.execute(QuarantineMemberRequest {
        actor: actor.to_owned(),
        member,
    })?;
    Ok(done(match decided {
        Quarantine::AlreadyLost => "already_lost",
        Quarantine::NotLauncher => "not_launcher",
        Quarantine::GracePending => "grace_pending",
        Quarantine::Recorded => "recorded",
    }))
}

/// `null`, with whether the death was confirmed, and whose.
pub(super) fn confirmed_dead(
    confirm_member_dead: &ConfirmMemberDead,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [member, exit] = take(arguments)?;
    let decided = confirm_member_dead.execute(ConfirmMemberDeadRequest {
        actor: actor.to_owned(),
        member,
        exit,
    })?;
    Ok(done(match decided {
        DeathConfirmation::AlreadyDead => "already_dead",
        DeathConfirmation::Confirmed { coordinator: false } => "confirmed",
        DeathConfirmation::Confirmed { coordinator: true } => "coordinator_confirmed",
    }))
}

/// `{id, status, outcome, deadline, coordinator, lost}`: the run's
/// columns after the op, each as stored, and whether it recorded a loss.
pub(super) fn lose_coordinator(
    lose_coordinator: &LoseCoordinator,
    actor: &str,
) -> Result<Served, BoardError> {
    let loss = lose_coordinator.execute(LoseCoordinatorRequest {
        actor: actor.to_owned(),
    })?;
    let text = |value: Option<String>| value.map_or(Value::Null, Value::String);
    let mut answer = Map::new();
    answer.insert("id".to_owned(), text(loss.run.id));
    answer.insert("status".to_owned(), text(loss.run.status));
    answer.insert("outcome".to_owned(), text(loss.run.outcome));
    answer.insert("deadline".to_owned(), loss.run.deadline);
    answer.insert("coordinator".to_owned(), text(loss.run.coordinator));
    answer.insert("lost".to_owned(), Value::Bool(loss.lost));
    let mut served = done(if loss.lost { "lost" } else { "not_lost" });
    served.value = Value::Object(answer);
    Ok(served)
}
