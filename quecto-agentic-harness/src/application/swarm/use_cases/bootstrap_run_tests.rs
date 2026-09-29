use serde_json::json;

use super::BootstrapRun;
use crate::application::swarm::board_test_support::{
    BoardState, CounterIds, MemoryBoard, SteppingClock, running_board, stored_member,
};
use crate::application::swarm::dto::BootstrapRunRequest;
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::RunState;

fn request() -> BootstrapRunRequest {
    BootstrapRunRequest {
        member: "parent".to_owned(),
        pid: json!(42),
        started: json!("Mon 1"),
        socket: json!("/run/parent.sock"),
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
    assert_eq!(run.record.status, Some(RunState::SETUP));
    assert_eq!(run.record.deadline, 0.0);
    assert_eq!(run.record.member_limit, 10);
    assert_eq!(run.record.coordinator.as_deref(), Some("parent"));
    assert_eq!(run.contract.goal, "");
    assert_eq!(run.contract.constraints, json!([]));
    assert_eq!(run.contract.criteria, json!([]));
    assert_eq!(
        state.members,
        [stored_member(
            "parent",
            json!(second),
            "live",
            [
                json!(42),
                json!("Mon 1"),
                json!("/run/parent.sock"),
                json!(null)
            ],
        )]
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

/// Served over another repository (#2303 round-3 review M1), bootstrap
/// writes to that board, drawing ids from the source it was composed with.
#[test]
fn over_writes_the_given_board_with_the_composed_ids() {
    let composed_over = MemoryBoard::with(BoardState::default());
    let other = MemoryBoard::with(BoardState::default());
    let bootstrap = BootstrapRun::new(
        composed_over.clone(),
        SteppingClock::fixed(5.0),
        CounterIds::journalling(&composed_over.journal),
    );
    assert!(
        bootstrap
            .over(other.clone())
            .execute(request())
            .unwrap()
            .created
    );
    assert_eq!(other.transactions(), [true]);
    assert!(other.snapshot().run.is_some());
    assert!(composed_over.transactions().is_empty());
    assert!(composed_over.snapshot().run.is_none());
    assert_eq!(
        composed_over.journal()[0],
        format!("draw {:032x}", 1),
        "the composed id source drew the run id"
    );
}
