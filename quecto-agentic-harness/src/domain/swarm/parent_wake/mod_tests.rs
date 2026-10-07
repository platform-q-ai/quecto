use super::*;

fn board(status: RunStatus, ready: i64, claimed: i64, idle_workers: i64) -> CoordinatorBoard {
    CoordinatorBoard {
        status,
        outcome: None,
        ready,
        claimed,
        submitted: 0,
        idle_workers,
    }
}

const LIVE: [RunStatus; 2] = [RunStatus::Setup, RunStatus::Running];
const TERMINAL: [RunStatus; 5] = [
    RunStatus::Succeeded,
    RunStatus::Blocked,
    RunStatus::Failed,
    RunStatus::Cancelled,
    RunStatus::BudgetExhausted,
];

#[test]
fn an_unreadable_board_wakes_as_an_ordinary_turn_end() {
    assert_eq!(parent_wake(None), ParentWake::Unknown);
    assert_eq!(parent_wake(None).status(), None);
}

#[test]
fn a_claimed_task_holds_the_parent() {
    for status in LIVE {
        assert_eq!(
            parent_wake(Some(&board(status, 0, 1, 0))),
            ParentWake::Hold { status }
        );
    }
}

#[test]
fn submitted_work_awaiting_a_verdict_holds_the_parent() {
    let mut board = board(RunStatus::Running, 0, 0, 2);
    board.submitted = 1;
    assert_eq!(
        parent_wake(Some(&board)),
        ParentWake::Hold {
            status: RunStatus::Running
        }
    );
}

#[test]
fn ready_work_with_a_free_worker_holds_the_parent() {
    assert_eq!(
        parent_wake(Some(&board(RunStatus::Running, 2, 0, 1))),
        ParentWake::Hold {
            status: RunStatus::Running
        }
    );
}

#[test]
fn ready_work_no_worker_can_take_wakes_the_parent_as_idle() {
    assert_eq!(
        parent_wake(Some(&board(RunStatus::Running, 2, 0, 0))),
        ParentWake::Idle {
            status: RunStatus::Running
        }
    );
}

#[test]
fn nothing_in_flight_wakes_the_parent_as_idle() {
    for status in LIVE {
        assert_eq!(
            parent_wake(Some(&board(status, 0, 0, 3))),
            ParentWake::Idle { status }
        );
    }
}

#[test]
fn a_paused_run_with_an_outcome_wakes_the_parent_with_that_outcome() {
    for outcome in TERMINAL {
        let mut board = board(RunStatus::Paused, 0, 1, 0);
        board.outcome = Some(outcome);
        let wake = parent_wake(Some(&board));
        assert_eq!(wake, ParentWake::Finished { outcome }, "{outcome:?}");
        assert_eq!(wake.status(), Some(outcome));
    }
}

#[test]
fn a_paused_run_with_no_outcome_wakes_the_parent_as_paused_whatever_is_claimed() {
    for claimed in [0, 1] {
        let wake = parent_wake(Some(&board(RunStatus::Paused, 0, claimed, 0)));
        assert_eq!(wake, ParentWake::Paused);
        assert_eq!(wake.status(), Some(RunStatus::Paused));
    }
}

#[test]
fn a_terminal_status_wakes_the_parent_as_finished() {
    for status in TERMINAL {
        assert_eq!(
            parent_wake(Some(&board(status, 0, 1, 0))),
            ParentWake::Finished { outcome: status },
            "{status:?}"
        );
    }
}

#[test]
fn each_wake_names_its_kind() {
    let running = RunStatus::Running;
    assert_eq!(ParentWake::Hold { status: running }.kind(), WakeKind::Hold);
    assert_eq!(ParentWake::Idle { status: running }.kind(), WakeKind::Idle);
    let finished = ParentWake::Finished {
        outcome: RunStatus::Succeeded,
    };
    assert_eq!(finished.kind(), WakeKind::Finished);
    assert_eq!(ParentWake::Paused.kind(), WakeKind::Paused);
    assert_eq!(ParentWake::Unknown.kind(), WakeKind::Unknown);
}

#[test]
fn a_held_note_settles_by_the_wake() {
    use HeldNote::*;
    let end = |prompted, errored| StretchEnd { prompted, errored };
    let cases = [
        (WakeKind::Hold, end(false, false), Silent),
        (WakeKind::Hold, end(true, false), Ordinary),
        (WakeKind::Hold, end(false, true), Stopped),
        (WakeKind::Hold, end(true, true), Stopped),
        (WakeKind::Unknown, end(false, false), Ordinary),
        (WakeKind::Unknown, end(true, false), Ordinary),
        (WakeKind::Unknown, end(false, true), Stopped),
    ];
    for (kind, end, note) in cases {
        assert_eq!(settle_held_note(kind, end), note, "{kind:?} {end:?}");
    }
    for prompted in [false, true] {
        for errored in [false, true] {
            let end = end(prompted, errored);
            assert_eq!(settle_held_note(WakeKind::Finished, end), Finished);
            assert_eq!(settle_held_note(WakeKind::Idle, end), Idle);
            assert_eq!(settle_held_note(WakeKind::Paused, end), Paused);
        }
    }
}

#[test]
fn a_wake_kind_this_build_does_not_know_reads_as_unknown() {
    let kind: WakeKind = serde_json::from_str("\"stalled\"").expect("decodes");
    assert_eq!(kind, WakeKind::Unknown);
    let hold: WakeKind = serde_json::from_str("\"hold\"").expect("decodes");
    assert_eq!(hold, WakeKind::Hold);
}

#[test]
fn a_claimed_task_with_every_worker_idle_is_stalled() {
    for status in LIVE {
        assert_eq!(
            workers_stalled(&board(status, 0, 2, 1)),
            Some(WorkersStalled {
                claimed: 2,
                ready: 0
            })
        );
    }
}

#[test]
fn ready_work_with_no_free_worker_is_stalled() {
    assert_eq!(
        workers_stalled(&board(RunStatus::Running, 3, 0, 0)),
        Some(WorkersStalled {
            claimed: 0,
            ready: 3
        })
    );
}

#[test]
fn ready_work_a_free_worker_was_woken_for_is_not_stalled() {
    assert_eq!(workers_stalled(&board(RunStatus::Running, 3, 0, 1)), None);
}

#[test]
fn work_awaiting_the_coordinator_or_none_left_is_not_stalled() {
    let mut submitted = board(RunStatus::Running, 0, 0, 2);
    submitted.submitted = 1;
    assert_eq!(workers_stalled(&submitted), None);
    assert_eq!(workers_stalled(&board(RunStatus::Running, 0, 0, 2)), None);
}

#[test]
fn a_run_that_is_not_live_is_never_stalled() {
    for status in TERMINAL.into_iter().chain([RunStatus::Paused]) {
        assert_eq!(workers_stalled(&board(status, 2, 2, 0)), None, "{status:?}");
    }
}
