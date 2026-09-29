//! #2277 review M1: the decision and detail each answered op records, for
//! the per-op table of `every_answered_op_records_what_it_acted_on`.
use std::sync::Arc;

use serde_json::json;

use super::super::super::{Method, SwarmBoardHandles, call};
use super::super::{Recorded, create_args, logged, only};
use crate::domain::swarm::{BoardOpDetail, BoardOpObservation, MemberExit, RunStatusKind};

/// The decision and detail a call of `method`, set up as
/// `ops::acted_on` sets it up, records. The `match` is exhaustive, so a
/// new method does not compile until what it records is stated here.
/// `None` for a decision is any kind the dispatcher names: only the ops
/// whose decisions carry detail pin theirs.
pub(super) fn decided(method: Method) -> (Option<&'static str>, BoardOpDetail) {
    // The loss and death records find the bootstrapped run in setup.
    let found = BoardOpDetail {
        run_status: Some(RunStatusKind::Setup),
        ..BoardOpDetail::NONE
    };
    match method {
        // The parent launched the worker and observes it first: the
        // grace runs from now, and nothing ends the run.
        Method::Quarantine => (Some("grace_pending"), found),
        Method::ConfirmedDead => (
            Some("confirmed"),
            BoardOpDetail {
                exit: Some(MemberExit::Orderly),
                reservations_retained: Some(0),
                ended_by_loss: Some(false),
                ..found
            },
        ),
        Method::LoseCoordinator => (
            Some("lost"),
            BoardOpDetail {
                ended_by_loss: Some(true),
                ..found
            },
        ),
        Method::Status
        | Method::Snapshot
        | Method::Admit
        | Method::Activate
        | Method::RecordLaunch
        | Method::ReleaseUnlaunched
        | Method::Socket
        | Method::TaskCreate
        | Method::Dependencies
        | Method::Claim
        | Method::Release
        | Method::Block
        | Method::Unblock
        | Method::Submit
        | Method::VerifyTask
        | Method::Pause
        | Method::Resume
        | Method::ResumeExternal
        | Method::Close
        | Method::ExtendDeadline
        | Method::Stop
        | Method::ControlStatus
        | Method::UsageReport
        | Method::Complete
        | Method::RevalidateTask
        | Method::Amend
        | Method::Evidence
        | Method::UsageBudget
        | Method::RecordRequest
        | Method::RequestAdmission
        | Method::Reserve
        | Method::ReleaseFiles
        | Method::FileOwners
        | Method::Recover
        | Method::Revoke
        | Method::Send
        | Method::Withdraw
        | Method::Inbox
        | Method::Ack
        | Method::Notifications
        | Method::AcceptWake
        | Method::Summary
        | Method::Events
        | Method::Task
        | Method::Tasks
        | Method::Create
        | Method::Bootstrap
        | Method::Join
        | Method::CreateRun
        | Method::BootstrapRun
        | Method::BootstrapJoin
        | Method::TaskRaw => (None, BoardOpDetail::NONE),
    }
}

/// `method` called by `member` on `handles` with `args`, answered; what it
/// recorded.
fn recorded(
    log: &Recorded,
    handles: &SwarmBoardHandles,
    member: &str,
    method: &str,
    args: serde_json::Value,
) -> BoardOpObservation {
    log.clear();
    call(handles, member, method, args).unwrap();
    only(log)
}

/// A running run with the worker admitted and activated by the parent,
/// holding task 1 and a reservation of `a.rs` on it.
fn reserving(handles: &SwarmBoardHandles) {
    call(handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    call(handles, "parent", "create_run", create_args()).unwrap();
    call(handles, "parent", "_admit", json!(["worker", "r1"])).unwrap();
    call(
        handles,
        "parent",
        "_activate",
        json!(["worker", "r1", 7, "s", "/w.sock"]),
    )
    .unwrap();
    call(
        handles,
        "parent",
        "task_create",
        json!(["r1", "t", ["tests pass"]]),
    )
    .unwrap();
    let token = call(handles, "worker", "claim", json!([1])).unwrap()["token"].clone();
    call(handles, "worker", "reserve", json!([1, token, ["a.rs"]])).unwrap();
}

/// The detail follows the decision: an abrupt death retains the
/// reservation and keeps the run; the launcherless coordinator's loss is
/// recorded at once and ends the run, which a later observation finds
/// paused; another member's observation of a launched member is refused
/// authority; the coordinator's confirmed death ends the run by loss.
#[test]
fn loss_decisions_record_what_they_found_and_did() {
    let log = Arc::new(Recorded::default());
    let (_dir, handles) = logged(&log);
    reserving(&handles);
    let running = BoardOpDetail {
        run_status: Some(RunStatusKind::Running),
        ..BoardOpDetail::NONE
    };
    let paused = BoardOpDetail {
        run_status: Some(RunStatusKind::Paused),
        ..BoardOpDetail::NONE
    };
    call(&handles, "parent", "_admit", json!(["other", "r2"])).unwrap();
    let observed = recorded(&log, &handles, "other", "_quarantine", json!(["worker"]));
    assert_eq!(
        (observed.decision.as_deref(), observed.detail),
        (Some("not_launcher"), running.clone())
    );
    let abrupt = recorded(
        &log,
        &handles,
        "parent",
        "_confirmed_dead",
        json!(["worker", "abrupt"]),
    );
    assert_eq!(abrupt.decision.as_deref(), Some("confirmed"));
    assert_eq!(
        abrupt.detail,
        BoardOpDetail {
            exit: Some(MemberExit::Abrupt),
            reservations_retained: Some(1),
            ended_by_loss: Some(false),
            ..running.clone()
        }
    );
    let quarantined = recorded(&log, &handles, "other", "_quarantine", json!(["parent"]));
    assert_eq!(
        (quarantined.decision.as_deref(), quarantined.detail),
        (
            Some("recorded"),
            BoardOpDetail {
                ended_by_loss: Some(true),
                ..running
            }
        )
    );
    let again = recorded(&log, &handles, "other", "_quarantine", json!(["parent"]));
    assert_eq!(
        (again.decision.as_deref(), again.detail),
        (Some("already_lost"), paused.clone())
    );
    let dead = recorded(
        &log,
        &handles,
        "other",
        "_confirmed_dead",
        json!(["worker"]),
    );
    assert_eq!(
        (dead.decision.as_deref(), dead.detail),
        (
            Some("already_dead"),
            BoardOpDetail {
                exit: Some(MemberExit::Orderly),
                ..paused.clone()
            }
        )
    );
    let lost = recorded(&log, &handles, "other", "_lose_coordinator", json!([]));
    assert_eq!(
        (lost.decision.as_deref(), lost.detail),
        (
            Some("not_lost"),
            BoardOpDetail {
                ended_by_loss: Some(false),
                ..paused
            }
        )
    );
}

/// The coordinator's confirmed death ends the running run by loss.
#[test]
fn a_coordinators_confirmed_death_records_the_end_by_loss() {
    let log = Arc::new(Recorded::default());
    let (_dir, handles) = logged(&log);
    reserving(&handles);
    let confirmed = recorded(
        &log,
        &handles,
        "worker",
        "_confirmed_dead",
        json!(["parent"]),
    );
    assert_eq!(
        (confirmed.decision.as_deref(), confirmed.detail),
        (
            Some("coordinator_confirmed"),
            BoardOpDetail {
                run_status: Some(RunStatusKind::Running),
                exit: Some(MemberExit::Orderly),
                reservations_retained: Some(0),
                ended_by_loss: Some(true),
            }
        )
    );
}
