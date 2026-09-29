use std::sync::Arc;

use serde_json::{Value, json};

use super::{ActivateMember, AdmitMember, JoinRun};
use crate::application::swarm::board_test_support::{
    BoardState, CounterIds, MemoryBoard, SteppingClock, running_board, stored_member,
};
use crate::application::swarm::dto::{JoinRunRequest, Joined, LaunchIdentity};
use crate::domain::swarm::BoardError;

fn join_run(board: &Arc<MemoryBoard>) -> JoinRun {
    let clock = SteppingClock::fixed(50.0);
    JoinRun::new(
        board.clone(),
        CounterIds::journalling(&board.journal),
        Arc::new(AdmitMember::new(board.clone(), clock.clone())),
        Arc::new(ActivateMember::new(board.clone(), clock)),
    )
}

fn join(member: &str, reservation: Option<&str>, pid: i64, started: &str) -> JoinRunRequest {
    JoinRunRequest {
        member: member.to_owned(),
        reservation: reservation.map_or(Value::Null, Value::from),
        launch: LaunchIdentity {
            pid: json!(pid),
            started: json!(started),
        },
        socket: json!(format!("/{member}.sock")),
    }
}

fn five_member_board() -> BoardState {
    let mut state = running_board(100.0);
    if let Some(run) = state.run.as_mut() {
        run.record.member_limit = 5;
    }
    state
}

/// A new identity is admitted and activated **as the coordinator**: the
/// coordinator is its launcher and the actor of both events, and the
/// reservation is drawn when none was given.
#[test]
fn join_acts_as_the_coordinator_and_records_it_as_launcher() {
    let board = MemoryBoard::with(five_member_board());
    assert_eq!(
        join_run(&board)
            .execute(join("worker", None, 7, "Mon"))
            .unwrap(),
        Joined::Admitted
    );
    let drawn = format!("{:032x}", 1);
    let state = board.snapshot();
    assert_eq!(
        state.members[1],
        stored_member(
            "worker",
            json!(drawn),
            "live",
            [
                json!(7),
                json!("Mon"),
                json!("/worker.sock"),
                json!("parent")
            ]
        )
    );
    let events: Vec<(&str, &str)> = state
        .events
        .iter()
        .map(|event| (event.actor.as_str(), event.action.as_str()))
        .collect();
    assert_eq!(events, [("parent", "reserved"), ("parent", "activated")]);
    assert_eq!(
        board.journal()[0],
        format!("draw {drawn}"),
        "drawn before admission"
    );
    // A given reservation is used as it is; an empty one is drawn afresh
    // (Python's `reservation or uuid4().hex`).
    let board = MemoryBoard::with(five_member_board());
    join_run(&board)
        .execute(join("given", Some("mine"), 7, "Mon"))
        .unwrap();
    join_run(&board)
        .execute(join("empty", Some(""), 8, "Mon"))
        .unwrap();
    let state = board.snapshot();
    assert_eq!(state.members[1].get("reservation"), Some(&json!("mine")));
    assert_eq!(
        state.members[2].get("reservation"),
        Some(&json!(format!("{:032x}", 1)))
    );
}

#[test]
fn join_returns_early_for_the_same_live_process() {
    let mut state = five_member_board();
    state.members.push(stored_member(
        "worker",
        json!("r"),
        "live",
        [json!(7), json!("Mon"), json!("/old"), json!("parent")],
    ));
    let board = MemoryBoard::with(state);
    // The reservation is not consulted for the same live process.
    assert_eq!(
        join_run(&board)
            .execute(join("worker", Some("other"), 7, "Mon"))
            .unwrap(),
        Joined::AlreadyLive {
            coordinator: Some("parent".to_owned())
        },
        "the coordinator the join read, for S12's summary gate"
    );
    assert_eq!(
        board.transactions(),
        [false],
        "one plain read, nothing written"
    );
    assert!(board.snapshot().events.is_empty());
    // Another process under the member's reservation is re-activated as
    // the coordinator, and is then held to the live process.
    assert_eq!(
        join_run(&board)
            .execute(join("worker", Some("r"), 8, "Mon"))
            .unwrap_err(),
        BoardError::new("member already active in a different process")
    );
}

#[test]
fn a_known_identity_under_another_reservation_is_refused() {
    let mut state = five_member_board();
    state.members.push(stored_member(
        "worker",
        json!("r"),
        "reserved",
        [json!(null), json!(null), json!(null), json!("parent")],
    ));
    let board = MemoryBoard::with(state);
    for reservation in [Some("other"), None] {
        assert_eq!(
            join_run(&board)
                .execute(join("worker", reservation, 7, "Mon"))
                .unwrap_err(),
            BoardError::new("launch reservation does not match invoking process")
        );
    }
    assert_eq!(
        join_run(&board)
            .execute(join("worker", Some("r"), 7, "Mon"))
            .unwrap(),
        Joined::Reactivated
    );
    let state = board.snapshot();
    assert_eq!(state.members[1].text("status"), Some("live"));
    assert_eq!(state.events[0].actor, "parent");
}

#[test]
fn join_without_a_run_or_coordinator_is_refused() {
    let board = MemoryBoard::with(BoardState::default());
    assert_eq!(
        join_run(&board)
            .execute(join("worker", None, 7, "Mon"))
            .unwrap_err(),
        BoardError::new("coordination run missing")
    );
    let mut state = five_member_board();
    if let Some(run) = state.run.as_mut() {
        run.record.coordinator = None;
    }
    let board = MemoryBoard::with(state);
    assert_eq!(
        join_run(&board)
            .execute(join("worker", None, 7, "Mon"))
            .unwrap_err(),
        BoardError::new("invoking member is unknown or death confirmed"),
        "nobody coordinates: the gate finds no invoking member"
    );
    // The already-live branch writes nothing and carries the coordinator
    // it read, nobody here, for S12's summary gate to refuse as.
    let mut state = five_member_board();
    if let Some(run) = state.run.as_mut() {
        run.record.coordinator = None;
    }
    state.members.push(stored_member(
        "live",
        json!("r"),
        "live",
        [json!(7), json!("Mon"), json!(null), json!("parent")],
    ));
    let live = MemoryBoard::with(state);
    assert_eq!(
        join_run(&live)
            .execute(join("live", None, 7, "Mon"))
            .unwrap(),
        Joined::AlreadyLive { coordinator: None }
    );
    let long: String = "x".repeat(129);
    assert_eq!(
        join_run(&board)
            .execute(join(&long, None, 7, "Mon"))
            .unwrap_err(),
        BoardError::new("member must be nonempty and at most 128 bytes"),
        "the member is bounded first"
    );
}

/// The join takes Python's `reservation or uuid4().hex` over any value
/// (#2271 round-1 review M1): every falsy one draws a fresh reservation,
/// and a truthy one is admitted as given, then compared by Python's `==`
/// when the coordinator activates it.
#[test]
fn join_takes_pythons_truthiness_and_equality_over_loose_values() {
    let mut state = five_member_board();
    if let Some(run) = state.run.as_mut() {
        run.record.member_limit = 10;
    }
    let board = MemoryBoard::with(state);
    let joining = join_run(&board);
    for (index, falsy) in [json!(0), json!(false), json!(0.0), json!([])]
        .into_iter()
        .enumerate()
    {
        let member = format!("falsy-{index}");
        let mut request = join(&member, None, 7, "Mon");
        request.reservation = falsy.clone();
        assert_eq!(
            joining.execute(request).unwrap(),
            Joined::Admitted,
            "{falsy}"
        );
        let row = board.snapshot().members.last().cloned().unwrap();
        assert_eq!(
            row.get("reservation"),
            Some(&json!(format!("{:032x}", index + 1))),
            "{falsy}"
        );
    }
    // `5` is admitted, stored as '5', and '5' != 5: the activation that
    // follows finds it stale, after the admission committed.
    let mut request = join("numeric", None, 8, "Mon");
    request.reservation = json!(5);
    assert_eq!(
        joining.execute(request).unwrap_err(),
        BoardError::new("unknown or stale launch reservation")
    );
    let row = board.snapshot().members.last().cloned().unwrap();
    assert_eq!(
        (row.get("id"), row.get("reservation"), row.text("status")),
        (Some(&json!("numeric")), Some(&json!("5")), Some("reserved"))
    );
    // The same live process compares by Python's `==`: `7.0 == 7`.
    let mut request = join("falsy-0", None, 7, "Mon");
    request.launch.pid = json!(7.0);
    assert_eq!(
        joining.execute(request).unwrap(),
        Joined::AlreadyLive {
            coordinator: Some("parent".to_owned())
        }
    );
}
