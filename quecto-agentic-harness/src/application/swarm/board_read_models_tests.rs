use serde_json::{Value, json};

use super::{page_bounds, summary, with_owner_liveness};
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, RecordedEvent, SteppingClock, member_row, running_board, stored_task,
};
use crate::application::swarm::dto::{RunSummary, SummaryScan, TaskRow};
use crate::application::swarm::ports::BoardRepository;
use crate::domain::swarm::{BoardError, RefusalKind};

fn at(actor: &str, time: f64, action: &str, member: &str) -> RecordedEvent {
    RecordedEvent {
        actor: actor.to_owned(),
        time,
        action: action.to_owned(),
        detail: json!({"member": member}),
    }
}

/// `worker` (active at 100), `quiet` (last seen at 0), `gone` (dead),
/// `lost` (recorded lost after its activation) and `later` (reserved)
/// each own a claimed task; task 6 is ready.
fn board() -> BoardState {
    let mut state = running_board(10_000.0);
    for (id, status) in [
        ("worker", "live"),
        ("quiet", "live"),
        ("gone", "dead"),
        ("lost", "live"),
        ("later", "reserved"),
    ] {
        state.members.push(member_row(id, status));
    }
    state.events = vec![
        at("worker", 100.0, "claimed", "worker"),
        at("quiet", 0.0, "claimed", "quiet"),
        at("parent", 1.0, "activated", "lost"),
        at("parent", 2.0, "scope_unknown", "lost"),
    ];
    for (id, owner) in [
        (1, "worker"),
        (2, "quiet"),
        (3, "gone"),
        (4, "lost"),
        (5, "later"),
    ] {
        state
            .tasks
            .push(stored_task(id, "claimed", json!([]), Some(owner)));
    }
    state.tasks.push(stored_task(6, "ready", json!([]), None));
    state
}

fn liveness(board: &MemoryBoard, now: f64) -> Vec<TaskRow> {
    let mut tasks = board.snapshot().tasks;
    board
        .atomic(false, &mut |transaction| {
            with_owner_liveness(transaction, &*SteppingClock::fixed(now), &mut tasks).map(|_| ())
        })
        .unwrap();
    tasks
}

/// Each owner's state and how to reach it, from one grouped scan of the
/// owners' events for the whole page (a counting double: the journal
/// notes every scan); an unowned task carries none of it.
#[test]
fn a_page_reads_its_owners_liveness_in_one_grouped_scan() {
    let board = MemoryBoard::with(board());
    let tasks = liveness(&board, 350.0);
    let scans = board
        .journal()
        .iter()
        .filter(|entry| entry.starts_with("latest_activity"))
        .count();
    assert_eq!(scans, 1, "{:?}", board.journal());
    let field = |task: &TaskRow, column: &str| task.get(column).cloned();
    let expected = [
        (
            "active",
            Some(json!(250.0)),
            Some(json!("board.send(request, 'worker', body)")),
            None,
        ),
        (
            "idle",
            Some(json!(350.0)),
            Some(json!("board.send(request, 'quiet', body)")),
            None,
        ),
        (
            "dead",
            None,
            Some(Value::Null),
            Some(json!("recover(task) or revoke(task, reason)")),
        ),
        (
            "lost",
            None,
            Some(Value::Null),
            Some(json!(
                "resume the run (agent_cmd swarm_control resume), then revoke(task, reason)"
            )),
        ),
        (
            "reserved",
            None,
            Some(Value::Null),
            Some(json!("revoke(task, reason)")),
        ),
    ];
    for (task, (state, activity, contact, recovery)) in tasks.iter().zip(expected) {
        assert_eq!(field(task, "owner_state"), Some(json!(state)), "{task:?}");
        assert_eq!(
            field(task, "owner_last_activity"),
            Some(activity.unwrap_or(Value::Null)),
            "{state}"
        );
        assert_eq!(field(task, "contact"), contact, "{state}");
        assert_eq!(field(task, "recovery"), recovery, "{state}");
    }
    for column in ["owner_last_activity", "owner_state", "contact", "recovery"] {
        assert!(tasks[5].get(column).is_none(), "{column}");
    }
}

/// A reader whose clock is behind the owner's last event reads no
/// negative activity (Python's `max(0.0, now - last)`), and a page with
/// no held claim reads nothing.
#[test]
fn activity_is_never_negative_and_an_unowned_page_reads_nothing() {
    let board = MemoryBoard::with(board());
    let tasks = liveness(&board, 50.0);
    assert_eq!(tasks[0].get("owner_last_activity"), Some(&json!(0.0)));
    let mut state = running_board(10_000.0);
    state.tasks.push(stored_task(1, "ready", json!([]), None));
    let empty = MemoryBoard::with(state);
    let tasks = liveness(&empty, 50.0);
    assert_eq!(tasks, empty.snapshot().tasks);
    assert!(empty.journal().is_empty());
}

fn summarised(state: BoardState, now: f64, since: Option<i64>) -> Result<RunSummary, BoardError> {
    let board = MemoryBoard::with(state);
    summary(&*board, &*SteppingClock::fixed(now), Some("parent"), since)
}

/// The fast path: the board's cursor, with no owner turned idle since the
/// cursor's event, answers `unchanged` with when to look again; an owner
/// turned idle by the clock alone defeats it.
#[test]
fn the_fast_path_answers_unchanged_until_an_owner_turns_idle() {
    let mut state = running_board(10_000.0);
    state.members.push(member_row("worker", "live"));
    state
        .tasks
        .push(stored_task(1, "claimed", json!([]), Some("worker")));
    state.events.push(at("worker", 100.0, "claimed", "worker"));
    let unchanged = summarised(state.clone(), 399.0, Some(1)).unwrap();
    assert_eq!(
        unchanged,
        RunSummary::Unchanged {
            event_cursor: 1,
            status: json!("running"),
            next_liveness_check_at: Some(400.0),
            scan: SummaryScan {
                owners_scanned: 1,
                fast_path_defeated: Some(false),
                cursor_moved: Some(false),
            },
        }
    );
    let RunSummary::Full(crossed) = summarised(state.clone(), 400.0, Some(1)).unwrap() else {
        panic!("an owner turned idle by the clock alone");
    };
    assert_eq!(crossed.next_liveness_check_at, None);
    assert_eq!(crossed.event_cursor, 1);
    assert_eq!(
        crossed.scan,
        SummaryScan {
            owners_scanned: 2,
            fast_path_defeated: Some(true),
            cursor_moved: Some(false),
        },
        "the watch's owner and the page's; the cursor held"
    );
    let RunSummary::Full(full) = summarised(state, 399.0, None).unwrap() else {
        panic!("no cursor given");
    };
    assert_eq!(
        (full.usage, full.task_count, full.counts.claimed),
        (2, 1, 1)
    );
}

/// The counts: a ready task with an incomplete (or missing) dependency is
/// blocked; a status the board never writes is refused naming the record.
#[test]
fn counts_derive_blocked_work_and_refuse_an_unknown_status() {
    let mut state = running_board(10_000.0);
    state.tasks = vec![
        stored_task(1, "completed", json!([]), None),
        stored_task(2, "ready", json!([1]), None),
        stored_task(3, "ready", json!([2]), None),
        stored_task(4, "ready", json!([99]), None),
        stored_task(5, "submitted", json!([]), Some("parent")),
    ];
    let RunSummary::Full(full) = summarised(state.clone(), 10.0, None).unwrap() else {
        panic!("no cursor given");
    };
    let counts = full.counts;
    assert_eq!(
        (
            counts.ready,
            counts.blocked,
            counts.submitted,
            counts.completed
        ),
        (1, 2, 1, 1)
    );
    state.tasks.push(stored_task(6, "weird", json!([]), None));
    assert_eq!(
        summarised(state, 10.0, None).unwrap_err(),
        BoardError::new(
            RefusalKind::Store,
            "the board's task status is not as the board writes it"
        )
    );
}

/// The counts at the caps (#2277 final review L1): a full board of 1000
/// tasks, 900 ready each naming 100 completed dependencies at the end of
/// the board, is counted in bounded time. A linear search per dependency
/// makes some 8.6e7 comparisons here; Python's dict makes 90 000 lookups.
#[test]
fn counts_at_the_caps_take_bounded_time() {
    let mut state = running_board(10_000.0);
    let dependencies = json!((901..=1000).collect::<Vec<i64>>());
    state.tasks = (1..=1000)
        .map(|id| match id {
            ..=900 => stored_task(id, "ready", dependencies.clone(), None),
            _ => stored_task(id, "completed", json!([]), None),
        })
        .collect();
    let started = std::time::Instant::now();
    let RunSummary::Full(full) = summarised(state, 10.0, None).unwrap() else {
        panic!("no cursor given");
    };
    let elapsed = started.elapsed();
    assert_eq!(
        (
            full.counts.ready,
            full.counts.blocked,
            full.counts.completed
        ),
        (900, 0, 100)
    );
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "counted in {elapsed:?}"
    );
}

/// A summary as nobody (a coordinator that is NULL) is refused by the
/// gate; a board without a run by its run check.
#[test]
fn nobodys_summary_is_refused_by_the_gate() {
    let board = MemoryBoard::with(running_board(100.0));
    assert_eq!(
        summary(&*board, &*SteppingClock::fixed(1.0), None, None).unwrap_err(),
        BoardError::new(
            RefusalKind::NotMember,
            "invoking member is unknown or death confirmed"
        )
    );
    let empty = MemoryBoard::with(BoardState::default());
    assert_eq!(
        summary(&*empty, &*SteppingClock::fixed(1.0), None, None)
            .unwrap_err()
            .kind(),
        RefusalKind::RunMissing
    );
}

/// Python's `type(x) is int` bounds: an offset of at least 0, a limit
/// from 1 to 100, never a boolean, a float or text.
#[test]
fn page_bounds_take_integers_only() {
    assert_eq!(page_bounds(&json!(0), &json!(100)), Some((0, 100)));
    for (offset, limit) in [
        (json!(-1), json!(1)),
        (json!(0), json!(0)),
        (json!(0), json!(101)),
        (json!(true), json!(1)),
        (json!(0), json!(1.0)),
        (json!("0"), json!(1)),
    ] {
        assert_eq!(page_bounds(&offset, &limit), None, "{offset} {limit}");
    }
}
