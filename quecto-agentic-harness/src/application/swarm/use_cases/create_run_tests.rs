use std::sync::Arc;

use serde_json::{Value, json};

use super::CreateRun;
use crate::application::swarm::board_test_support::{
    BoardState, CompactEncoding, CounterIds, MemoryBoard, SteppingClock, member_row, running_board,
};
use crate::application::swarm::dto::{CreateBranch, CreateRunRequest};
use crate::application::swarm::ports::BoardEncoding;
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind, RunState};

const NOW: f64 = 1_000.0;
const WEEK: f64 = 604_800.0;

fn create_run(board: &Arc<MemoryBoard>, clock: Arc<SteppingClock>) -> CreateRun {
    CreateRun::new(
        board.clone(),
        clock,
        CounterIds::journalling(&board.journal),
        Arc::new(CompactEncoding),
    )
}

fn request(member: &str) -> CreateRunRequest {
    CreateRunRequest {
        member: member.to_owned(),
        goal: json!("ship the slice"),
        constraints: json!(["no shortcuts"]),
        criteria: json!([
            {"id": "tests", "kind": "command", "description": "cargo test"},
            {"id": "review", "kind": "review", "description": "two cold reviews"},
        ]),
        member_limit: json!(3),
        deadline: json!(NOW + 3_600.0),
    }
}

fn with(change: impl FnOnce(&mut CreateRunRequest)) -> CreateRunRequest {
    let mut request = request("parent");
    change(&mut request);
    request
}

/// Each row breaks one rule, and every later rule too where it can, so the
/// message shows which rule Python checks first. No row reaches the store.
#[test]
fn create_validations_fire_in_python_order() {
    let goal_text = "goal must be nonempty and at most 8192 bytes";
    let constraints_size = "constraints must be nonempty and at most 8192 bytes";
    let criteria_required = "explicit evidence criteria required";
    let kinds = "criteria distinguish command checks from parent-reviewed requirements";
    let constraints_shape = "constraints must be a list of strings";
    let limit = "member limit must be 1 through 25 including coordinator";
    let deadline = "deadline must be in the next seven days";
    let everything_wrong = |goal: Value| {
        with(|request| {
            request.goal = goal;
            request.constraints = json!(["x".repeat(9_000)]);
            request.criteria = json!([]);
            request.member_limit = json!(0);
            request.deadline = json!(0);
        })
    };
    let rows: Vec<(CreateRunRequest, &str)> = vec![
        (everything_wrong(json!(5)), goal_text),
        (everything_wrong(json!(" \t")), goal_text),
        (everything_wrong(json!("g".repeat(8_193))), goal_text),
        (everything_wrong(json!("goal")), constraints_size),
        (
            with(|request| {
                request.constraints = json!("not a list");
                request.criteria = json!([]);
                request.member_limit = json!(0);
            }),
            criteria_required,
        ),
        (
            with(|request| {
                request.constraints = json!(7);
                request.criteria = json!([{"id": "a", "kind": "bogus", "description": "d"}]);
            }),
            kinds,
        ),
        (
            with(|request| {
                request.criteria = json!([
                    {"id": "a", "kind": "command", "description": "d"},
                    {"id": "a", "kind": "review", "description": "e"},
                ]);
                request.member_limit = json!(0);
            }),
            "duplicate criterion id",
        ),
        (
            with(|request| {
                request.criteria =
                    json!([{"id": "a", "kind": "command", "description": "d".repeat(16_400)}]);
            }),
            "criterion description must be nonempty and at most 8192 bytes",
        ),
        (
            with(|request| {
                request.constraints = json!("not a list");
                request.member_limit = json!(0);
            }),
            constraints_shape,
        ),
        (
            with(|request| request.constraints = json!(["ok", 1])),
            constraints_shape,
        ),
        (
            with(|request| {
                request.member_limit = json!(0);
                request.deadline = json!(0);
            }),
            limit,
        ),
        (with(|request| request.member_limit = json!(26)), limit),
        (with(|request| request.member_limit = json!(true)), limit),
        (with(|request| request.member_limit = json!(3.0)), limit),
        (with(|request| request.member_limit = json!("3")), limit),
        (with(|request| request.member_limit = Value::Null), limit),
        (with(|request| request.deadline = json!(NOW)), deadline),
        (
            with(|request| request.deadline = json!(NOW - 1.0)),
            deadline,
        ),
        (
            with(|request| request.deadline = json!(NOW + WEEK + 0.5)),
            deadline,
        ),
        (
            with(|request| request.deadline = json!(NOW + 8.0 * 86_400.0)),
            deadline,
        ),
        (with(|request| request.deadline = json!("soon")), deadline),
        (with(|request| request.deadline = Value::Null), deadline),
        (with(|request| request.deadline = json!(true)), deadline),
    ];
    for (request, expected) in rows {
        let board = MemoryBoard::with(BoardState::default());
        let refused = create_run(&board, SteppingClock::fixed(NOW))
            .execute(request.clone())
            .unwrap_err();
        assert_eq!(
            refused,
            BoardError::new(RefusalKind::Invalid, expected),
            "{request:?}"
        );
        assert!(
            board.transactions().is_empty(),
            "a refused contract never opens the store: {request:?}"
        );
    }
}

#[test]
fn create_accepts_the_window_edges_and_loosely_typed_numbers() {
    for deadline in [json!(NOW + WEEK), json!(NOW + 0.001), json!(1_001)] {
        let board = MemoryBoard::with(BoardState::default());
        let created = create_run(&board, SteppingClock::fixed(NOW))
            .execute(with(|request| request.deadline = deadline.clone()));
        assert!(created.is_ok(), "{deadline}: {created:?}");
    }
    // A boolean is an int to `isinstance`: `True` is the deadline 1.
    let board = MemoryBoard::with(BoardState::default());
    create_run(&board, SteppingClock::fixed(0.5))
        .execute(with(|request| request.deadline = json!(true)))
        .unwrap();
    assert_eq!(board.snapshot().run.unwrap().record.deadline, 1.0);
    for limit in [1, 25] {
        let board = MemoryBoard::with(BoardState::default());
        create_run(&board, SteppingClock::fixed(NOW))
            .execute(with(|request| request.member_limit = json!(limit)))
            .unwrap();
        assert_eq!(board.snapshot().run.unwrap().record.member_limit, limit);
    }
}

/// The upper bound is read on the clock only once the lower bound holds,
/// as Python's chained comparison reads `time.time()`.
#[test]
fn the_deadline_window_reads_the_clock_as_python_does() {
    // Read 1: 1000 < deadline. Read 2: deadline <= 1000 + WEEK fails only
    // if the second reading is the one used for the upper bound.
    let board = MemoryBoard::with(BoardState::default());
    let refused = create_run(&board, SteppingClock::new(&[NOW, NOW - 10.0]))
        .execute(with(|request| request.deadline = json!(NOW + WEEK - 5.0)))
        .unwrap_err();
    assert_eq!(
        refused,
        BoardError::new(
            RefusalKind::Invalid,
            "deadline must be in the next seven days"
        )
    );
}

#[test]
fn create_draws_run_id_before_reservation() {
    let board = MemoryBoard::with(BoardState::default());
    let created = create_run(&board, SteppingClock::fixed(NOW))
        .execute(request("parent"))
        .unwrap();
    assert_eq!(created.branch, CreateBranch::Fresh);
    let first = format!("{:032x}", 1);
    let second = format!("{:032x}", 2);
    assert_eq!(
        board.journal(),
        [
            format!("draw {first}"),
            format!("insert_run {first}"),
            format!("draw {second}"),
            format!("insert_member {second}"),
            "event created".to_owned(),
        ]
    );
    // One creating transaction, committed; then the creator's summary
    // through the read-only gate's two (#2277).
    assert_eq!(board.transactions(), [true, false, false]);
    let state = board.snapshot();
    let run = state.run.unwrap();
    assert_eq!(run.id, first);
    assert_eq!(run.record.status, Some(RunState::RUNNING));
    assert_eq!(run.record.coordinator.as_deref(), Some("parent"));
    assert_eq!(run.record.member_limit, 3);
    assert_eq!(state.members.len(), 1);
    assert_eq!(state.members[0].text("status"), Some("live"));
    assert_eq!(state.members[0].text("reservation"), Some(second.as_str()));
}

/// The `created` event records the contract as the member gave it: an
/// integer deadline stays an integer in the event, while the run stores it
/// as seconds.
#[test]
fn the_created_event_records_the_contract_as_given() {
    let board = MemoryBoard::with(BoardState::default());
    create_run(&board, SteppingClock::fixed(NOW))
        .execute(with(|request| request.deadline = json!(4_600)))
        .unwrap();
    let state = board.snapshot();
    let event = &state.events[0];
    assert_eq!(
        (event.actor.as_str(), event.time, event.action.as_str()),
        ("parent", NOW, "created")
    );
    let original = request("parent");
    assert_eq!(
        event.detail,
        json!({
            "goal": "ship the slice",
            "deadline": 4_600,
            "contract": {
                "goal": "ship the slice",
                "constraints": original.constraints,
                "criteria": original.criteria,
            },
        })
    );
    assert!(event.detail["deadline"].is_i64());
    assert_eq!(state.run.unwrap().record.deadline, 4_600.0);
}

#[test]
fn only_the_setup_coordinator_can_create_over_an_existing_run() {
    let refusal = "only the setup coordinator can create this run; existing runs cannot be reset";
    let mut setup = running_board(0.0);
    setup.run.as_mut().unwrap().record.status = Some(RunState::SETUP);

    // Another member may not take the placeholder over.
    let board = MemoryBoard::with(setup.clone());
    let refused = create_run(&board, SteppingClock::fixed(NOW))
        .execute(request("worker"))
        .unwrap_err();
    assert_eq!(refused, BoardError::new(RefusalKind::RunExists, refusal));
    assert!(board.snapshot().events.is_empty());

    // A created run is never reset, not even by its coordinator.
    let board = MemoryBoard::with(running_board(NOW + 60.0));
    let refused = create_run(&board, SteppingClock::fixed(NOW))
        .execute(request("parent"))
        .unwrap_err();
    assert_eq!(refused, BoardError::new(RefusalKind::RunExists, refusal));

    // The placeholder's coordinator creates over it: the run keeps its id
    // and members, takes the contract and runs; no id is drawn.
    let board = MemoryBoard::with(setup);
    let created = create_run(&board, SteppingClock::fixed(NOW))
        .execute(request("parent"))
        .unwrap();
    assert_eq!(created.branch, CreateBranch::OverSetup);
    assert_eq!(
        board.journal(),
        ["update_run_contract", "event created"].map(str::to_owned)
    );
    let state = board.snapshot();
    let run = state.run.unwrap();
    assert_eq!(run.id, "run-1");
    assert_eq!(run.record.status, Some(RunState::RUNNING));
    assert_eq!(run.record.deadline, NOW + 3_600.0);
    assert_eq!(run.record.member_limit, 3);
    assert_eq!(run.contract.goal, "ship the slice");
    assert_eq!(state.members.len(), 1);
}

#[test]
fn members_not_confirmed_dead_must_fit_the_new_limit() {
    let mut setup = running_board(0.0);
    setup.run.as_mut().unwrap().record.status = Some(RunState::SETUP);
    setup.members.push(member_row("worker", "reserved"));
    setup.members.push(member_row("gone", "dead"));
    setup.members.push(member_row("odd", "unknown-status"));

    let board = MemoryBoard::with(setup.clone());
    let refused = create_run(&board, SteppingClock::fixed(NOW))
        .execute(with(|request| request.member_limit = json!(2)))
        .unwrap_err();
    assert_eq!(
        refused,
        BoardError::new(
            RefusalKind::MemberLimit,
            "existing live/reserved members exceed requested limit; terminate and reconcile first"
        )
    );
    assert_eq!(
        board.snapshot().run.unwrap().record.status,
        Some(RunState::SETUP)
    );

    // Three members are not dead; a limit of three fits them exactly.
    let board = MemoryBoard::with(setup);
    create_run(&board, SteppingClock::fixed(NOW))
        .execute(request("parent"))
        .unwrap();
    assert_eq!(
        board.snapshot().run.unwrap().record.status,
        Some(RunState::RUNNING)
    );
}

/// The size bound is on the encoded text, as the real codec writes it
/// (`ensure_ascii`): 1,400 `é` are 2,800 UTF-8 bytes but 8,400 escaped,
/// over the 8,192-byte bound, so the fake encoding escapes too.
#[test]
fn constraints_are_bounded_on_their_ascii_escaped_encoding() {
    let board = MemoryBoard::with(BoardState::default());
    let refused = create_run(&board, SteppingClock::fixed(NOW))
        .execute(with(|request| {
            request.constraints = json!(["é".repeat(1_400)]);
        }))
        .unwrap_err();
    assert_eq!(
        refused,
        BoardError::new(
            RefusalKind::Invalid,
            "constraints must be nonempty and at most 8192 bytes"
        )
    );
    let encoded = CompactEncoding
        .encode(&json!({"b": ["é😀"], "a": 1}))
        .unwrap();
    assert_eq!(encoded, r#"{"a":1,"b":["\u00e9\ud83d\ude00"]}"#);
}

/// Served over another repository (#2303 round-3 review M1), create writes
/// to that board, drawing ids from the source it was composed with.
#[test]
fn over_creates_on_the_given_board_with_the_composed_ports() {
    let composed_over = MemoryBoard::with(BoardState::default());
    let other = MemoryBoard::with(BoardState::default());
    let composed = create_run(&composed_over, SteppingClock::fixed(NOW));
    let created = composed.over(other.clone()).execute(request("parent"));
    assert_eq!(created.unwrap().branch, CreateBranch::Fresh);
    assert!(other.snapshot().run.is_some());
    assert!(composed_over.transactions().is_empty());
    assert!(composed_over.snapshot().run.is_none());
    assert!(
        composed_over
            .journal()
            .iter()
            .any(|entry| entry.starts_with("draw ")),
        "the composed id source drew the run's ids"
    );
}
