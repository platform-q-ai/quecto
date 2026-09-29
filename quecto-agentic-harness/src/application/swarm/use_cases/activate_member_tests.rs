use serde_json::{Value, json};

use super::ActivateMember;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, member_row, running_board, stored_member,
};
use crate::application::swarm::dto::{ActivateMemberRequest, LaunchIdentity};
use crate::domain::swarm::{BoardError, RunState};

fn launch(pid: i64, started: &str) -> LaunchIdentity {
    LaunchIdentity {
        pid: json!(pid),
        started: json!(started),
    }
}

fn activate(
    member: &str,
    reservation: Option<&str>,
    pid: i64,
    started: &str,
) -> ActivateMemberRequest {
    ActivateMemberRequest {
        actor: "parent".to_owned(),
        member: json!(member),
        reservation: reservation.map_or(Value::Null, Value::from),
        launch: launch(pid, started),
        socket: json!("/w.sock"),
    }
}

fn with_worker(status: &str, pid: Value, started: Value) -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(stored_member(
        "worker",
        json!("worker-reservation"),
        status,
        [pid, started, json!(null), json!("parent")],
    ));
    state
}

#[test]
fn a_reserved_member_is_activated_in_its_process() {
    let board = MemoryBoard::with(with_worker("reserved", json!(null), json!(null)));
    let service = ActivateMember::new(board.clone(), SteppingClock::fixed(50.0));
    service
        .execute(activate("worker", Some("worker-reservation"), 4242, "Mon"))
        .unwrap();
    let state = board.snapshot();
    assert_eq!(
        state.members[1],
        stored_member(
            "worker",
            json!("worker-reservation"),
            "live",
            [json!(4242), json!("Mon"), json!("/w.sock"), json!("parent")]
        )
    );
    let event = &state.events[0];
    assert_eq!(
        (event.actor.as_str(), event.action.as_str(), &event.detail),
        (
            "parent",
            "activated",
            &json!({"member": "worker", "pid": 4242})
        )
    );
}

/// The same process activating again is accepted (a join's retry); a live
/// member in another process is refused, and so is any stale reservation.
#[test]
fn launch_identity_and_reservation_are_checked() {
    let live = || with_worker("live", json!(1), json!("s"));
    let service = |state| ActivateMember::new(MemoryBoard::with(state), SteppingClock::fixed(50.0));
    service(live())
        .execute(activate("worker", Some("worker-reservation"), 1, "s"))
        .unwrap();
    for (pid, started) in [(2, "s"), (1, "t")] {
        assert_eq!(
            service(live())
                .execute(activate("worker", Some("worker-reservation"), pid, started))
                .unwrap_err(),
            BoardError::new("member already active in a different process")
        );
    }
    // A reserved member whose launch was recorded under another pid is
    // activated anyway: only a live member is held to its process.
    service(with_worker("reserved", json!(9), json!("x")))
        .execute(activate("worker", Some("worker-reservation"), 1, "s"))
        .unwrap();
    // A live member with no pid yet takes any process.
    service(with_worker("live", json!(null), json!(null)))
        .execute(activate("worker", Some("worker-reservation"), 1, "s"))
        .unwrap();
    let stale = BoardError::new("unknown or stale launch reservation");
    for (state, member, reservation) in [
        (live(), "worker", Some("other")),
        (live(), "worker", None),
        (live(), "stranger", Some("worker-reservation")),
        (
            with_worker("dead", json!(null), json!(null)),
            "worker",
            Some("worker-reservation"),
        ),
    ] {
        assert_eq!(
            service(state)
                .execute(activate(member, reservation, 1, "s"))
                .unwrap_err(),
            stale,
            "{member} {reservation:?}"
        );
    }
}

/// A stored pid equal as a number (`1.0`) is the same process, as Python's
/// `==` finds; a pid stored as text is not.
#[test]
fn a_stored_pid_compares_as_python_compares_it() {
    let service = |pid| {
        ActivateMember::new(
            MemoryBoard::with(with_worker("live", pid, json!("s"))),
            SteppingClock::fixed(50.0),
        )
    };
    service(json!(1.0))
        .execute(activate("worker", Some("worker-reservation"), 1, "s"))
        .unwrap();
    assert_eq!(
        service(json!("1"))
            .execute(activate("worker", Some("worker-reservation"), 1, "s"))
            .unwrap_err(),
        BoardError::new("member already active in a different process")
    );
}

/// A NULL stored reservation matches only an absent one (Python's
/// `None != None` is false).
#[test]
fn a_null_reservation_matches_only_an_absent_one() {
    let mut state = running_board(100.0);
    state.members.push(stored_member(
        "worker",
        json!(null),
        "reserved",
        [json!(null), json!(null), json!(null), json!(null)],
    ));
    let service = ActivateMember::new(MemoryBoard::with(state.clone()), SteppingClock::fixed(50.0));
    service.execute(activate("worker", None, 1, "s")).unwrap();
    let service = ActivateMember::new(MemoryBoard::with(state), SteppingClock::fixed(50.0));
    assert_eq!(
        service
            .execute(activate("worker", Some("x"), 1, "s"))
            .unwrap_err(),
        BoardError::new("unknown or stale launch reservation")
    );
}

/// Only a setup or running run activates. An expired run is ended first
/// (the first transaction commits) and then refused.
#[test]
fn a_stopped_or_expired_run_refuses_activation() {
    let mut paused = with_worker("reserved", json!(null), json!(null));
    if let Some(run) = paused.run.as_mut() {
        run.record.status = Some(RunState::PAUSED);
    }
    let refused = BoardError::new("run stopped before activation");
    let service = ActivateMember::new(MemoryBoard::with(paused), SteppingClock::fixed(50.0));
    assert_eq!(
        service
            .execute(activate("worker", Some("worker-reservation"), 1, "s"))
            .unwrap_err(),
        refused
    );
    let mut setup = with_worker("reserved", json!(null), json!(null));
    if let Some(run) = setup.run.as_mut() {
        run.record.status = Some(RunState::SETUP);
    }
    ActivateMember::new(MemoryBoard::with(setup), SteppingClock::fixed(50.0))
        .execute(activate("worker", Some("worker-reservation"), 1, "s"))
        .unwrap();
    let board = MemoryBoard::with(with_worker("reserved", json!(null), json!(null)));
    let service = ActivateMember::new(board.clone(), SteppingClock::fixed(100.0));
    assert_eq!(
        service
            .execute(activate("worker", Some("worker-reservation"), 1, "s"))
            .unwrap_err(),
        refused
    );
    let state = board.snapshot();
    assert_eq!(state.run.unwrap().record.status, Some(RunState::PAUSED));
    assert_eq!(state.events.len(), 2, "stop and paused committed");
    assert_eq!(state.members[1].text("status"), Some("reserved"));
}

/// The owner-decided divergence `unknown_member_status_is_not_alive`
/// (#2295, listed in `tests/integration/swarm_board_diff_loose.rs`):
/// Python activates a member whose status is anything but `'dead'`; Rust
/// activates only a `live` or `reserved` one, so a NULL or unknown status
/// is a stale reservation.
#[test]
fn an_unknown_member_status_is_not_activated() {
    for status in [json!(null), json!("weird"), json!("LIVE")] {
        let mut state = running_board(100.0);
        let mut row = member_row("worker", "reserved");
        row.columns[2].1 = status.clone();
        state.members.push(row);
        let service = ActivateMember::new(MemoryBoard::with(state), SteppingClock::fixed(50.0));
        assert_eq!(
            service
                .execute(activate("worker", Some("worker-reservation"), 1, "s"))
                .unwrap_err(),
            BoardError::new("unknown or stale launch reservation"),
            "{status}"
        );
    }
}
