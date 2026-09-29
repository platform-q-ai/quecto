//! The harness's membership methods of the board dispatch (#2271):
//! `_admit`, `_activate`, `_record_launch`, `_release_unlaunched` and
//! `_socket`, their Python signatures and their serving. Every argument
//! reaches the use case as the JSON value passed (#2271 round-1 review
//! M1). None acts on a task or a message, or moves a cursor (#2303).
use serde_json::Value;

use super::{Parameter, Served, done, member_row, required, take};
use crate::application::swarm::dto::{
    ActivateMemberRequest, AdmissionDecision, AdmitMemberRequest, LaunchIdentity,
    RecordMemberLaunchRequest, RegisterMemberSocketRequest, ReleaseUnlaunchedMemberRequest,
};
use crate::application::swarm::use_cases::{
    ActivateMember, AdmitMember, RecordMemberLaunch, RegisterMemberSocket, ReleaseUnlaunchedMember,
};
use crate::domain::swarm::{BoardError, BoardOpDetail};

pub(super) const ADMIT: [Parameter; 2] = [required("member"), required("reservation")];
pub(super) const ACTIVATE: [Parameter; 5] = [
    required("member"),
    required("reservation"),
    required("pid"),
    required("started"),
    required("socket"),
];
pub(super) const RECORD_LAUNCH: [Parameter; 4] = [
    required("member"),
    required("reservation"),
    required("pid"),
    required("started"),
];
pub(super) const RELEASE_UNLAUNCHED: [Parameter; 1] = [required("member")];
pub(super) const SOCKET: [Parameter; 1] = [required("socket")];

/// `_admit(member, reservation)`: the member's row, as `dict(row)`. The
/// member stays the JSON value passed, which the board bounds.
pub(super) fn admit(
    admit_member: &AdmitMember,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [member, reservation] = take(arguments)?;
    let admitted = admit_member.execute(AdmitMemberRequest {
        actor: actor.to_owned(),
        member,
        reservation,
    })?;
    Ok(Served {
        value: member_row(admitted.row),
        decision: match admitted.decision {
            AdmissionDecision::Reserved => "reserved",
            AdmissionDecision::Retry => "retry",
        },
        task_id: None,
        message_id: None,
        cursor_moved: None,
        detail: BoardOpDetail::NONE,
    })
}

pub(super) fn activate(
    activate_member: &ActivateMember,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [member, reservation, pid, started, socket] = take(arguments)?;
    activate_member.execute(ActivateMemberRequest {
        actor: actor.to_owned(),
        member,
        reservation,
        launch: LaunchIdentity { pid, started },
        socket,
    })?;
    Ok(done("activated"))
}

pub(super) fn record_launch(
    record_member_launch: &RecordMemberLaunch,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [member, reservation, pid, started] = take(arguments)?;
    record_member_launch.execute(RecordMemberLaunchRequest {
        actor: actor.to_owned(),
        member,
        reservation,
        launch: LaunchIdentity { pid, started },
    })?;
    Ok(done("recorded"))
}

pub(super) fn release_unlaunched(
    release_unlaunched_member: &ReleaseUnlaunchedMember,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [member] = take(arguments)?;
    release_unlaunched_member.execute(ReleaseUnlaunchedMemberRequest {
        actor: actor.to_owned(),
        member,
    })?;
    Ok(done("released"))
}

pub(super) fn socket(
    register_member_socket: &RegisterMemberSocket,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [socket] = take(arguments)?;
    register_member_socket.execute(RegisterMemberSocketRequest {
        actor: actor.to_owned(),
        socket,
    })?;
    Ok(done("registered"))
}
