use super::ReadBoardProjection;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, StoredFile, accepted, member_row, recorded, running_board,
    stored_task, usage,
};
use crate::application::swarm::dto::MemberRole;
use serde_json::{Value, json};

/// A running run: `parent` coordinates; `worker` holds item 1, which has a
/// reserved path; item 2 waits on item 1; item 3 is ready; item 4 is done.
fn board() -> BoardState {
    let mut state = running_board(500.0);
    state.members.push(member_row("worker", "live"));
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
        token: "reservation-token".into(),
    });
    let mut event = recorded("worker", "claimed", json!({"task": 1}));
    event.time = 42.5;
    state.events.push(event);
    state
        .evidence
        .push(accepted("parent", "tests", "abc123", "command"));
    state.usage = Some(usage(
        json!({"token_limit": 1000, "strict_unknown": true}),
        250,
        1,
    ));
    state
}

#[test]
fn the_projection_carries_the_run_items_members_evidence_and_totals() {
    let projection = ReadBoardProjection::new(MemoryBoard::with(board()))
        .execute()
        .unwrap();
    let run = projection.run.expect("a created run");
    assert_eq!(
        (run.id, run.goal, run.status),
        (json!("run-1"), json!("goal"), json!("running"))
    );
    assert_eq!(
        (run.member_limit, run.deadline, run.generation),
        (json!(2), json!(500.0), 0)
    );
    assert_eq!(run.budget["token_limit"], json!(1000));
    assert_eq!(
        (
            run.budget["observed_tokens"].clone(),
            run.budget["unknown_usage_requests"].clone()
        ),
        (json!(250), json!(1))
    );

    let items: Vec<(Value, Value, Value)> = projection
        .items
        .iter()
        .map(|item| (item.id.clone(), item.state.clone(), item.owner.clone()))
        .collect();
    assert_eq!(
        items,
        [
            (json!(1), json!("claimed"), json!("worker")),
            (json!(2), json!("blocked"), Value::Null),
            (json!(3), json!("ready"), Value::Null),
            (json!(4), json!("completed"), json!("parent")),
        ]
    );
    let first = &projection.items[0];
    assert_eq!(
        (first.criteria.clone(), first.depends_on.clone()),
        (json!(["pass"]), json!([]))
    );
    assert_eq!(first.reserved_paths, [json!("src/a.rs")]);
    assert_eq!(
        (
            projection.items[1].depends_on.clone(),
            projection.items[1].blocker.clone()
        ),
        (json!([1]), json!("unmet dependencies"))
    );

    let members: Vec<(Value, MemberRole, Vec<Value>, Value)> = projection
        .members
        .iter()
        .map(|member| {
            (
                member.id.clone(),
                member.role,
                member.claims.clone(),
                member.last_activity.clone(),
            )
        })
        .collect();
    assert_eq!(
        members,
        [
            (
                json!("parent"),
                MemberRole::Coordinator,
                vec![],
                Value::Null
            ),
            (
                json!("worker"),
                MemberRole::Worker,
                vec![json!(1)],
                json!(42.5)
            ),
        ]
    );
    assert_eq!(projection.evidence.len(), 1);
    assert_eq!(
        (
            projection.evidence[0].criterion.clone(),
            projection.evidence[0].actor.clone()
        ),
        (json!("tests"), json!("parent"))
    );
    assert_eq!(
        (
            projection.totals.claimed,
            projection.totals.ready,
            projection.totals.blocked
        ),
        (1, 1, 1)
    );
    assert_eq!(projection.totals.completed, 1);
}

#[test]
fn no_claim_or_reservation_token_crosses() {
    let projection = ReadBoardProjection::new(MemoryBoard::with(board()))
        .execute()
        .unwrap();
    let seen = format!("{projection:?}");
    assert!(
        !seen.contains("stored-token") && !seen.contains("reservation-token"),
        "{seen}"
    );
}

#[test]
fn a_board_without_a_run_projects_no_run_and_no_items() {
    let projection = ReadBoardProjection::new(MemoryBoard::with(BoardState::default()))
        .execute()
        .unwrap();
    assert_eq!(
        (projection.run, projection.items, projection.members),
        (None, vec![], vec![])
    );
}

#[test]
fn the_projection_is_one_transaction_that_never_creates_a_board() {
    let board = MemoryBoard::with(board());
    ReadBoardProjection::new(board.clone()).execute().unwrap();
    assert_eq!(
        board.transactions(),
        [false],
        "one transaction, never creating a board"
    );
    let state = board.snapshot();
    assert_eq!(
        (state.tasks.len(), state.files.len()),
        (4, 1),
        "nothing was written"
    );
}
