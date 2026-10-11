use super::{PROJECTION_LIMITS, ReadBoardProjection};
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, StoredFile, accepted, recorded, running_board, stored_member,
    stored_task, usage,
};
use crate::application::swarm::dto::{ItemState, MemberRole, NewRequestUsage};
use serde_json::json;

/// A running run: `parent` coordinates; `worker` holds item 1, which has a
/// reserved path; item 2 waits on item 1; item 3 is ready; item 4 is done.
/// Every column the projection must not carry holds a marker.
fn board() -> BoardState {
    let mut state = running_board(500.0);
    let secret = |name: &str| json!(format!("secret-{name}"));
    state.members.push(stored_member(
        "worker",
        secret("reservation"),
        "live",
        [
            json!(4242),
            json!(1.5),
            secret("socket"),
            secret("launcher"),
        ],
    ));
    state.tasks = vec![
        stored_task(1, "claimed", json!([]), Some("worker")),
        stored_task(2, "ready", json!([1]), None),
        stored_task(3, "ready", json!([]), None),
        stored_task(4, "completed", json!([]), Some("parent")),
    ];
    state.files.push(StoredFile {
        path: "src/a.rs".into(),
        task: 1,
        owner: "worker".into(),
        claim: "claim".into(),
        token: "secret-reservation-token".into(),
    });
    let mut event = recorded("worker", "claimed", json!({"task": 1}));
    event.time = 42.5;
    state.events.push(event);
    state
        .evidence
        .push(accepted("parent", "tests", "abc123", "command"));
    let budget =
        json!({"token_limit": 1000, "strict_unknown": true, "warned": false, "secret": "no"});
    state.usage = Some(usage(budget, 250, 1));
    state
}

fn project(state: BoardState) -> crate::application::swarm::dto::BoardProjection {
    ReadBoardProjection::new(MemoryBoard::with(state))
        .execute()
        .unwrap()
}

#[test]
fn the_projection_carries_the_run_items_members_evidence_and_totals() {
    let projection = project(board());
    let run = projection.run.expect("a created run");
    assert_eq!(
        (run.id.as_str(), run.goal.as_str(), run.status.as_str()),
        ("run-1", "goal", "running")
    );
    assert_eq!(
        (run.member_limit, run.deadline, run.generation),
        (2, 500.0, 0)
    );
    assert_eq!(run.coordinator.as_deref(), Some("parent"));
    let budget = run.budget;
    assert_eq!(
        (budget.token_limit, budget.strict_unknown, budget.warned),
        (Some(1000), true, false)
    );
    assert_eq!(
        (budget.observed_tokens, budget.unknown_usage_requests),
        (250, 1)
    );

    let items: Vec<(i64, ItemState, bool, Option<&str>)> = projection
        .items
        .iter()
        .map(|item| (item.id, item.state, item.waiting, item.owner.as_deref()))
        .collect();
    assert_eq!(
        items,
        [
            (1, ItemState::Claimed, false, Some("worker")),
            (2, ItemState::Ready, true, None),
            (3, ItemState::Ready, false, None),
            (4, ItemState::Completed, false, Some("parent")),
        ]
    );
    let first = &projection.items[0];
    assert_eq!(
        (first.criteria.clone(), first.reserved_paths.clone()),
        (vec!["pass".to_owned()], vec!["src/a.rs".to_owned()])
    );
    assert_eq!(
        (
            projection.items[1].depends_on.clone(),
            projection.items[1].blocker.clone()
        ),
        (vec![1], None)
    );

    let members: Vec<(&str, MemberRole, Vec<i64>, Option<f64>)> = projection
        .members
        .iter()
        .map(|member| {
            (
                member.id.as_str(),
                member.role,
                member.claims.clone(),
                member.last_activity,
            )
        })
        .collect();
    assert_eq!(
        members,
        [
            ("parent", MemberRole::Coordinator, vec![], None),
            ("worker", MemberRole::Worker, vec![1], Some(42.5))
        ]
    );
    assert_eq!(projection.evidence.len(), 1);
    let evidence = &projection.evidence[0];
    assert_eq!(
        (
            evidence.criterion.as_str(),
            evidence.actor.as_str(),
            evidence.accepted
        ),
        ("tests", "parent", true)
    );
    let totals = projection.totals;
    assert_eq!(
        (
            totals.items,
            totals.ready,
            totals.waiting,
            totals.claimed,
            totals.completed
        ),
        (4, 2, 1, 1, 1)
    );
    assert_eq!((totals.members, totals.members_without_claim), (2, 1));
    assert!(!projection.truncated);
}

#[test]
fn ready_maps_to_todo_and_each_state_to_its_project_board_state() {
    let mapped: Vec<&str> = [
        ItemState::Ready,
        ItemState::Claimed,
        ItemState::Blocked,
        ItemState::Submitted,
        ItemState::Completed,
    ]
    .iter()
    .map(|state| state.project_board_state())
    .collect();
    assert_eq!(mapped, ["todo", "doing", "blocked", "review", "done"]);
    assert_eq!(ItemState::parse("verified"), None);
}

#[test]
fn no_token_member_process_detail_or_budget_extra_crosses() {
    let seen = format!("{:?}", project(board()));
    assert!(!seen.contains("secret") && !seen.contains("4242"), "{seen}");
}

#[test]
fn a_board_without_a_run_projects_no_run_and_no_items() {
    let projection = project(BoardState::default());
    assert_eq!(
        (projection.run, projection.items, projection.members),
        (None, vec![], vec![])
    );
}

#[test]
fn the_projection_is_one_read_transaction() {
    let board = MemoryBoard::with(board());
    ReadBoardProjection::new(board.clone()).execute().unwrap();
    assert_eq!(
        (board.reads(), board.transactions()),
        (1, vec![]),
        "no write transaction"
    );
}

#[test]
fn recording_usage_moves_the_revision() {
    let board = MemoryBoard::with(board());
    let read = ReadBoardProjection::new(board.clone());
    let before = read.revision().unwrap();
    assert_eq!(read.execute().unwrap().revision, before);
    board
        .state
        .lock()
        .unwrap()
        .request_usage
        .push(NewRequestUsage {
            request_id: "r1".into(),
            actor: "worker".into(),
            payload: "{}".into(),
            tokens: 10,
            unknown: 0,
            attempts: 1,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        });
    assert!(read.revision().unwrap() > before);
}

/// One change to a valid board.
type Edit = fn(&mut BoardState);

#[test]
fn values_the_board_never_writes_are_refused() {
    let edits: Vec<(&str, Edit)> = vec![
        ("unknown state", |state| {
            state.tasks[0].columns[4].1 = json!("verified")
        }),
        ("state not text", |state| {
            state.tasks[0].columns[4].1 = json!(3)
        }),
        ("title not text", |state| {
            state.tasks[0].columns[1].1 = json!(["x"])
        }),
        ("criteria not a list", |state| {
            state.tasks[0].columns[2].1 = json!("pass")
        }),
        ("dependency not an id", |state| {
            state.tasks[1].columns[3].1 = json!(["one"])
        }),
        ("budget limit not a number", |state| {
            state.usage.as_mut().unwrap().budget = json!({"token_limit": "many"})
        }),
        ("accepted not a flag", |state| {
            state.evidence[0].1.accepted = json!("yes")
        }),
    ];
    for (case, edit) in edits {
        let mut state = board();
        edit(&mut state);
        let refused = ReadBoardProjection::new(MemoryBoard::with(state)).execute();
        assert!(refused.is_err(), "{case}: {refused:?}");
    }
}

#[test]
fn rows_past_the_byte_budget_are_left_out_and_said_to_be() {
    let mut state = board();
    state.tasks[2].columns[1].1 = json!("x".repeat(10_000));
    let projection = ReadBoardProjection::new(MemoryBoard::with(state))
        .within(5_000)
        .unwrap();
    let ids: Vec<i64> = projection.items.iter().map(|item| item.id).collect();
    assert_eq!(ids, [1, 2], "items stop at the first that does not fit");
    assert!(projection.truncated);
    assert_eq!(projection.totals.items, 4, "totals count every item");
}

/// L2 (#2488 review): the budget counts a row as the socket line encodes
/// it. A control character is one byte stored and six encoded (`\u0001`),
/// so a title that fits raw may not fit once encoded.
#[test]
fn rows_are_budgeted_by_their_encoded_length() {
    let mut state = board();
    state.tasks[2].columns[1].1 = json!("\u{1}".repeat(1_000));
    let projection = ReadBoardProjection::new(MemoryBoard::with(state))
        .within(5_000)
        .unwrap();
    let ids: Vec<i64> = projection.items.iter().map(|item| item.id).collect();
    assert_eq!(ids, [1, 2], "item 3 is 1,000 bytes raw but 6,002 encoded");
    assert!(projection.truncated);
}

#[test]
fn rows_past_a_row_limit_are_left_out_and_said_to_be() {
    let mut state = board();
    let more = PROJECTION_LIMITS.items + 1;
    state.tasks = (1..=more)
        .map(|id| stored_task(id, "ready", json!([]), None))
        .collect();
    let projection = project(state);
    assert_eq!(projection.items.len() as i64, PROJECTION_LIMITS.items);
    assert_eq!(projection.totals.items, more);
    assert!(projection.truncated);
}
