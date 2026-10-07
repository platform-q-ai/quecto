use super::*;

fn board(status: &str, ready: i64, claimed: i64, idle_workers: i64) -> CoordinatorBoard {
    CoordinatorBoard {
        status: status.to_owned(),
        ready,
        claimed,
        idle_workers,
    }
}

#[test]
fn an_unreadable_board_wakes_as_an_ordinary_turn_end() {
    assert_eq!(parent_wake(None), ParentWake::Unknown);
}

#[test]
fn a_claimed_task_holds_the_parent() {
    assert_eq!(
        parent_wake(Some(&board("running", 0, 1, 0))),
        ParentWake::Hold
    );
}

#[test]
fn ready_work_with_a_free_worker_holds_the_parent() {
    assert_eq!(
        parent_wake(Some(&board("running", 2, 0, 1))),
        ParentWake::Hold
    );
}

#[test]
fn ready_work_no_worker_can_take_wakes_the_parent_as_idle() {
    assert_eq!(
        parent_wake(Some(&board("running", 2, 0, 0))),
        ParentWake::Idle
    );
}

#[test]
fn nothing_in_flight_wakes_the_parent_as_idle() {
    assert_eq!(
        parent_wake(Some(&board("running", 0, 0, 3))),
        ParentWake::Idle
    );
}

#[test]
fn any_status_but_running_wakes_the_parent_as_finished() {
    for status in [
        "complete",
        "stopped",
        "paused",
        "failed",
        "lost",
        "budget-exhausted",
        "setup",
    ] {
        assert_eq!(
            parent_wake(Some(&board(status, 0, 1, 0))),
            ParentWake::Finished {
                status: status.to_owned()
            },
            "{status}"
        );
    }
}
