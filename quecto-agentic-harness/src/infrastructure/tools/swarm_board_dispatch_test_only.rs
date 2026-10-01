//! The test-only methods of the board dispatch (#2270, #2271): `create_run`
//! (`create` without its summary), `bootstrap_run` and `bootstrap_join`
//! (`_bootstrap`'s halves, without the summary), their signatures and
//! their serving. Mounted only in `test` and `test-support` builds.
use serde_json::Value;

use super::{Parameter, Served, done, required, take};
use crate::application::swarm::dto::{BootstrapRunRequest, JoinRunRequest, Joined, LaunchIdentity};
use crate::application::swarm::use_cases::{BootstrapRun, CreateRun, JoinRun};
use crate::domain::swarm::{BoardError, BoardOpDetail};

pub(super) const BOOTSTRAP: [Parameter; 3] =
    [required("pid"), required("started"), required("socket")];

/// `create` without its summary: `null`, whatever the summary answered.
pub(super) fn create_run(
    create_run: &CreateRun,
    member: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let created = super::reads::created(create_run, member, arguments)?;
    Ok(Served {
        controls_run: true,
        ..done(super::reads::branch(&created))
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
        detail: BoardOpDetail::NONE,
        refused: None,
        controls_run: false,
    })
}

/// `join_process` for the calling member without the coordinator's
/// closing summary: `null`, as the driver alias answers.
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
