//! #2277 review M1 and M2: the decision and detail each answered op
//! records, for the per-op table of
//! `every_answered_op_records_what_it_acted_on`, and the detail of the
//! loss ops and the read models across their decisions.
use std::sync::Arc;

use serde_json::json;

use super::super::super::{Method, SwarmBoardHandles, call};
use super::super::{Recorded, create_args, logged, only};
use crate::domain::swarm::{
    BoardOpDetail, BoardOpObservation, BoardOpOutcome, MemberExit, RefusalKind, RunStatusKind,
};

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
        // #2277 review M2: the read models over the bootstrapped run,
        // which holds no task and the placeholder's one event.
        Method::Summary => (
            Some("full"),
            BoardOpDetail {
                owners_scanned: Some(0),
                page_size: Some(0),
                ..BoardOpDetail::NONE
            },
        ),
        Method::Tasks => (
            Some("read"),
            BoardOpDetail {
                owners_scanned: Some(0),
                page_size: Some(0),
                ..BoardOpDetail::NONE
            },
        ),
        Method::Events => (
            Some("read"),
            BoardOpDetail {
                page_size: Some(1),
                has_more: Some(false),
                ..BoardOpDetail::NONE
            },
        ),
        Method::Create => (Some("over_setup"), BoardOpDetail::NONE),
        Method::Bootstrap => (
            Some("already_live"),
            BoardOpDetail {
                placeholder_created: Some(false),
                ..BoardOpDetail::NONE
            },
        ),
        Method::Join => (Some("already_live"), BoardOpDetail::NONE),
        // #2338: the watch's tick, given no cursor, answers the snapshot.
        Method::Watch => (Some("snapshot"), BoardOpDetail::NONE),
        Method::Status
        | Method::EventCursor
        | Method::Snapshot
        | Method::RunTotals
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
        | Method::Task
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
                ..BoardOpDetail::NONE
            }
        )
    );
}

/// A running run whose worker claimed task 1 and whose parent created
/// task 2, on a board file in the returned directory.
fn claimed(log: &Arc<Recorded>) -> (tempfile::TempDir, SwarmBoardHandles) {
    let (dir, handles) = logged(log);
    reserving(&handles);
    call(
        &handles,
        "parent",
        "task_create",
        json!(["r2", "t", ["tests pass"]]),
    )
    .unwrap();
    (dir, handles)
}

/// `summary` (#2277 review M2): the full summary counts the owners its
/// watch and page read and the tasks it holds; the board's own cursor
/// answers `unchanged`, with the cursor unmoved and the fast path held;
/// an older cursor moved; an owner turned idle by the clock alone since
/// the board's cursor defeats the fast path.
#[test]
fn a_summary_records_its_scan_and_its_fast_path() {
    let log = Arc::new(Recorded::default());
    let (dir, handles) = claimed(&log);
    let full = recorded(&log, &handles, "parent", "summary", json!([]));
    let cursor = call(&handles, "parent", "summary", json!([])).unwrap()["event_cursor"].clone();
    assert_eq!(
        (full.decision.as_deref(), full.cursor_moved, full.detail),
        (
            Some("full"),
            None,
            BoardOpDetail {
                owners_scanned: Some(2),
                page_size: Some(2),
                ..BoardOpDetail::NONE
            }
        ),
        "the watch's owner and the page's"
    );
    let unchanged = recorded(&log, &handles, "parent", "summary", json!([cursor]));
    assert_eq!(
        (
            unchanged.decision.as_deref(),
            unchanged.cursor_moved,
            unchanged.detail
        ),
        (
            Some("unchanged"),
            Some(false),
            BoardOpDetail {
                owners_scanned: Some(1),
                fast_path_defeated: Some(false),
                ..BoardOpDetail::NONE
            }
        )
    );
    let behind = recorded(&log, &handles, "parent", "summary", json!([1]));
    assert_eq!(
        (
            behind.decision.as_deref(),
            behind.cursor_moved,
            behind.detail.fast_path_defeated
        ),
        (Some("full"), Some(true), None)
    );
    // The worker's events, the cursor's among them, go back to 600: idle
    // by the clock (1000) alone, and active at the cursor's time.
    rusqlite::Connection::open(dir.path().join("swarm.sqlite"))
        .unwrap()
        .execute(
            "UPDATE events SET time=600 WHERE actor='worker' OR id=?",
            [cursor.as_i64().unwrap()],
        )
        .unwrap();
    let crossed = recorded(&log, &handles, "parent", "summary", json!([cursor]));
    assert_eq!(
        (
            crossed.decision.as_deref(),
            crossed.cursor_moved,
            crossed.detail.fast_path_defeated
        ),
        (Some("full"), Some(false), Some(true))
    );
}

/// `tasks` and `events` (#2277 review M2) record their page's size, the
/// owners `tasks` read, and whether `events` has more.
#[test]
fn pages_record_their_size() {
    let log = Arc::new(Recorded::default());
    let (_dir, handles) = claimed(&log);
    let page = recorded(&log, &handles, "parent", "tasks", json!([0, 1]));
    assert_eq!(
        page.detail,
        BoardOpDetail {
            owners_scanned: Some(1),
            page_size: Some(1),
            ..BoardOpDetail::NONE
        }
    );
    let events = recorded(&log, &handles, "parent", "events", json!([0, 3]));
    assert_eq!(
        (events.detail.page_size, events.detail.has_more),
        (Some(3), Some(true))
    );
}

/// `create` over a board whose creator's row is gone (#2277 review M2)
/// commits the run and then refuses: its record says so, with the branch
/// it took; `_bootstrap` of a fresh board records the placeholder it
/// wrote (whose creator the join then finds already live).
#[test]
fn a_committed_create_refusal_differs_from_a_plain_one() {
    let log = Arc::new(Recorded::default());
    let (dir, handles) = logged(&log);
    let bootstrapped = recorded(
        &log,
        &handles,
        "parent",
        "_bootstrap",
        json!([1, "s", null]),
    );
    assert_eq!(
        (
            bootstrapped.decision.as_deref(),
            bootstrapped.detail.placeholder_created
        ),
        (Some("already_live"), Some(true))
    );
    rusqlite::Connection::open(dir.path().join("swarm.sqlite"))
        .unwrap()
        .execute("DELETE FROM members WHERE id='parent'", [])
        .unwrap();
    log.clear();
    call(&handles, "parent", "create", create_args()).unwrap_err();
    let committed = only(&log);
    assert_eq!(
        (
            committed.outcome,
            committed.decision.as_deref(),
            committed.result_bytes
        ),
        (
            BoardOpOutcome::Refused {
                kind: RefusalKind::NotMember,
                committed: true
            },
            Some("over_setup"),
            0
        )
    );
    log.clear();
    call(&handles, "parent", "create", create_args()).unwrap_err();
    let plain = only(&log);
    assert_eq!(
        (plain.outcome, plain.decision),
        (
            BoardOpOutcome::Refused {
                kind: RefusalKind::RunExists,
                committed: false
            },
            None
        )
    );
}
