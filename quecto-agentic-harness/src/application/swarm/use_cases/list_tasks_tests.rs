use serde_json::json;

use super::ListTasks;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, member_row, recorded, running_board, stored_task,
};
use crate::application::swarm::dto::ListTasksRequest;
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind};

fn page(offset: serde_json::Value, limit: serde_json::Value) -> ListTasksRequest {
    ListTasksRequest {
        actor: "parent".to_owned(),
        offset,
        limit,
    }
}

fn board() -> std::sync::Arc<MemoryBoard> {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    state.events.push(recorded("worker", "claimed", json!({})));
    for id in 1..=3 {
        state
            .tasks
            .push(stored_task(id, "claimed", json!([]), Some("worker")));
    }
    MemoryBoard::with(state)
}

/// A page of tasks by id, each with its owner's liveness, read in one
/// grouped scan.
#[test]
fn a_page_carries_its_owners_liveness() {
    let board = board();
    let tasks = ListTasks::new(board.clone(), SteppingClock::fixed(10.0))
        .execute(page(json!(1), json!(5)))
        .unwrap();
    assert_eq!(tasks.owners_scanned, 2, "one owner per owned task");
    let tasks = tasks.tasks;
    let ids: Vec<_> = tasks.iter().map(|task| task.get("id").cloned()).collect();
    assert_eq!(ids, [Some(json!(2)), Some(json!(3))]);
    assert!(
        tasks
            .iter()
            .all(|task| task.text("owner_state") == Some("active"))
    );
    let scans = board
        .journal()
        .iter()
        .filter(|entry| entry.starts_with("latest_activity"))
        .count();
    assert_eq!(scans, 1);
}

#[test]
fn the_bounds_are_checked_before_the_gate() {
    let board = board();
    let service = ListTasks::new(board.clone(), SteppingClock::fixed(10.0));
    for (offset, limit) in [
        (json!(-1), json!(1)),
        (json!(0), json!(1000)),
        (json!(0), json!(false)),
    ] {
        assert_eq!(
            service.execute(page(offset, limit)).unwrap_err(),
            BoardError::new(
                RefusalKind::Invalid,
                "task page requires nonnegative offset and limit 1 through 100"
            )
        );
    }
    assert!(board.transactions().is_empty());
}

#[test]
fn over_reads_the_given_board() {
    let composed_over = board();
    ListTasks::new(composed_over.clone(), SteppingClock::fixed(10.0))
        .over(board())
        .execute(page(json!(0), json!(50)))
        .unwrap();
    assert!(composed_over.transactions().is_empty());
}
