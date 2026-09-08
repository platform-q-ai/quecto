use super::swarm_lifecycle::{Board, Observation};
use std::sync::Mutex;
#[test]
fn observing_harness_death_quarantines_but_never_releases_ownership() {
    let board = Board(Mutex::new(vec![]));
    quecto::application::swarm::reconcile(&board, &Observation).unwrap();
    assert_eq!(*board.0.lock().unwrap(), ["worker"]);
}
