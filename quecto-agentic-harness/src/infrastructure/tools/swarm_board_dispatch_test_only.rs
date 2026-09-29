//! The test-only methods of the board dispatch (#2270, #2271):
//! `create_run`, `bootstrap_run` and `bootstrap_join`, their signatures and
//! their serving. Mounted only in `test` and `test-support` builds.
use serde_json::Value;

use super::{Parameter, Served, done, required, take};
use crate::application::swarm::dto::{
    BootstrapRunRequest, CreateBranch, CreateRunRequest, JoinRunRequest, Joined, LaunchIdentity,
};
use crate::application::swarm::use_cases::{BootstrapRun, CreateRun, JoinRun};
use crate::domain::swarm::BoardError;

pub(super) const CREATE: [Parameter; 5] = [
    required("goal"),
    required("constraints"),
    required("criteria"),
    required("member_limit"),
    required("deadline"),
];
pub(super) const BOOTSTRAP: [Parameter; 3] =
    [required("pid"), required("started"), required("socket")];
/// `_bootstrap(pid, started, socket, reservation=None)`'s signature.
pub(super) const JOIN: [Parameter; 4] = [
    required("pid"),
    required("started"),
    required("socket"),
    Parameter {
        name: "reservation",
        default: Some(|| Value::Null),
    },
];

pub(super) fn create_run(
    create_run: &CreateRun,
    member: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [goal, constraints, criteria, member_limit, deadline] = take(arguments)?;
    let created = create_run.execute(CreateRunRequest {
        member: member.to_owned(),
        goal,
        constraints,
        criteria,
        member_limit,
        deadline,
    })?;
    Ok(Served {
        value: Value::Null,
        decision: match created.branch {
            CreateBranch::Fresh => "fresh",
            CreateBranch::OverSetup => "over_setup",
        },
        task_id: None,
        message_id: None,
        cursor_moved: None,
    })
}

/// The three values reach the store as the member passed them: Python
/// binds them untyped (epic P3), so a type is never refused here.
pub(super) fn bootstrap_run(
    bootstrap_run: &BootstrapRun,
    member: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [pid, started, socket] = take(arguments)?;
    let bootstrapped = bootstrap_run.execute(BootstrapRunRequest {
        member: member.to_owned(),
        pid,
        started,
        socket,
    })?;
    Ok(Served {
        value: Value::Null,
        decision: if bootstrapped.created {
            "created"
        } else {
            "existing"
        },
        task_id: None,
        message_id: None,
        cursor_moved: None,
    })
}

/// `join_process` for the calling member: `null`, as the driver alias
/// answers until S12 adds the summary.
pub(super) fn bootstrap_join(
    join_run: &JoinRun,
    member: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [pid, started, socket, reservation] = take(arguments)?;
    let joined = join_run.execute(JoinRunRequest {
        member: member.to_owned(),
        reservation,
        launch: LaunchIdentity { pid, started },
        socket,
    })?;
    Ok(done(match joined {
        Joined::Admitted => "admitted",
        Joined::AlreadyLive { .. } => "already_live",
        Joined::Reactivated => "reactivated",
    }))
}
