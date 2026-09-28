use serde_json::json;

use super::BootstrapRun;
use crate::application::swarm::dto::{BootstrapRunRequest, MemberRow};
use crate::application::swarm::use_cases::fakes::{
    BoardState, CounterIds, MemoryBoard, SteppingClock, running_board,
};
use crate::domain::swarm::RunState;

fn request() -> BootstrapRunRequest {
    BootstrapRunRequest {
        member: "parent".to_owned(),
        pid: Some(42),
        started: Some("Mon 1".to_owned()),
        socket: Some("/run/parent.sock".to_owned()),
    }
}

#[test]
fn bootstrap_writes_the_setup_placeholder_once() {
    let board = MemoryBoard::with(BoardState::default());
    let bootstrap = BootstrapRun::new(
        board.clone(),
        SteppingClock::fixed(5.0),
        CounterIds::journalling(&board.journal),
    );
    assert!(bootstrap.execute(request()).unwrap().created);
    let first = format!("{:032x}", 1);
    let second = format!("{:032x}", 2);
    assert_eq!(
        board.journal(),
        [
            format!("draw {first}"),
            format!("insert_run {first}"),
            format!("draw {second}"),
            format!("insert_member {second}"),
            "event container_setup".to_owned(),
        ]
    );
    assert_eq!(board.transactions(), [true], "bootstrap creates the store");
    let state = board.snapshot();
    let run = state.run.unwrap();
    assert_eq!(run.record.status, RunState::SETUP);
    assert_eq!(run.record.deadline, 0.0);
    assert_eq!(run.record.member_limit, 10);
    assert_eq!(run.record.coordinator, "parent");
    assert_eq!(run.contract.goal, "");
    assert_eq!(run.contract.constraints, json!([]));
    assert_eq!(run.contract.criteria, json!([]));
    assert_eq!(
        state.members,
        [MemberRow {
            id: "parent".to_owned(),
            reservation: Some(second),
            status: "live".to_owned(),
            pid: Some(42),
            started: Some("Mon 1".to_owned()),
            socket: Some("/run/parent.sock".to_owned()),
            launcher: None,
        }]
    );
    let event = &state.events[0];
    assert_eq!(
        (event.actor.as_str(), event.time, event.action.as_str()),
        ("parent", 5.0, "container_setup")
    );
    assert_eq!(event.detail, json!({"member": "parent"}));

    // A board that holds a run is left as it is.
    let again = bootstrap.execute(request()).unwrap();
    assert!(!again.created);
    assert_eq!(board.snapshot().events.len(), 1);
}

#[test]
fn bootstrap_leaves_a_created_run_untouched() {
    let board = MemoryBoard::with(running_board(100.0));
    let bootstrap = BootstrapRun::new(
        board.clone(),
        SteppingClock::fixed(5.0),
        CounterIds::journalling(&board.journal),
    );
    assert!(!bootstrap.execute(request()).unwrap().created);
    assert!(
        board.journal().is_empty(),
        "no id is drawn, nothing written"
    );
}
