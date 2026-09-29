//! The loss and death methods of the board dispatch (#2277, #1924,
//! #1961): the harness's `_quarantine`, `_confirmed_dead` and
//! `_lose_coordinator`, their Python signatures and their serving. The
//! member and the exit kind reach the use cases as the JSON values passed
//! (Python binds the member untyped and checks the kind at run time). A
//! record names no task, message or member beyond the caller; its
//! decision says what the op decided, and its detail (#2277 review M1)
//! the run status the op found, the exit kind, the reservations a death
//! left held and whether the run ended by loss: kinds and counts only.
use serde_json::{Map, Value};

use super::{Parameter, Served, done, required, take};
use crate::application::swarm::dto::{
    ConfirmMemberDeadRequest, DeathConfirmation, LoseCoordinatorRequest, Quarantine,
    QuarantineMemberRequest,
};
use crate::application::swarm::use_cases::{ConfirmMemberDead, LoseCoordinator, QuarantineMember};
use crate::domain::swarm::{BoardError, BoardOpDetail, RunState, RunStatusKind};

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
    let (decision, ended_by_loss) = match decided.decision {
        Quarantine::AlreadyLost => ("already_lost", None),
        Quarantine::NotLauncher => ("not_launcher", None),
        Quarantine::GracePending => ("grace_pending", None),
        Quarantine::Recorded { ended_by_loss } => ("recorded", Some(ended_by_loss)),
    };
    Ok(detailed(
        decision,
        BoardOpDetail {
            ended_by_loss,
            ..found(decided.run_status.as_ref())
        },
    ))
}

/// The detail of an op that found the run in `status`, and nothing else.
fn found(status: Option<&RunState>) -> BoardOpDetail {
    BoardOpDetail {
        run_status: Some(RunStatusKind::of(status)),
        ..BoardOpDetail::NONE
    }
}

/// `null`, with `decision` and its `detail`.
fn detailed(decision: &'static str, detail: BoardOpDetail) -> Served {
    let mut served = done(decision);
    served.detail = detail;
    served
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
    let (decision, reservations_retained, ended_by_loss) = match decided.decision {
        DeathConfirmation::AlreadyDead => ("already_dead", None, None),
        DeathConfirmation::Confirmed {
            coordinator,
            reservations_retained,
            ended_by_loss,
        } => (
            if coordinator {
                "coordinator_confirmed"
            } else {
                "confirmed"
            },
            Some(reservations_retained),
            Some(ended_by_loss),
        ),
    };
    Ok(detailed(
        decision,
        BoardOpDetail {
            exit: Some(decided.exit),
            reservations_retained,
            ended_by_loss,
            ..found(decided.run_status.as_ref())
        },
    ))
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
    let mut served = detailed(
        if loss.lost { "lost" } else { "not_lost" },
        BoardOpDetail {
            ended_by_loss: Some(loss.lost),
            ..found(loss.found.as_ref())
        },
    );
    served.value = Value::Object(answer);
    Ok(served)
}
