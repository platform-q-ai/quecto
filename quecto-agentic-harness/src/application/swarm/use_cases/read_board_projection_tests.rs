use super::{PROJECTION_LIMITS, ReadBoardProjection};
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, StoredFile, accepted, recorded, running_board, stored_member,
    stored_task, usage,
};
use crate::application::swarm::dto::{MemberRole, NewRequestUsage};
use crate::domain::swarm::{BoardRole, MemberState, RefusalKind, RunState, TaskState};
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
        (run.id.as_str(), run.goal.as_str(), run.status),
        ("run-1", "goal", RunState::RUNNING)
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

    let items: Vec<(i64, TaskState, bool, Option<&str>)> = projection
        .items
        .iter()
        .map(|item| {
            (
                item.id,
                item.state.clone(),
                item.waiting,
                item.owner.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        items,
        [
            (1, TaskState::CLAIMED, false, Some("worker")),
            (2, TaskState::READY, true, None),
            (3, TaskState::READY, false, None),
            (4, TaskState::COMPLETED, false, Some("parent")),
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

    let members: Vec<MemberSeen> = projection
        .members
        .iter()
        .map(|member| {
            (
                member.id.as_str(),
                member.role,
                member.status.clone(),
                member.claims.clone(),
                member.last_activity,
            )
        })
        .collect();
    assert_eq!(
        members,
        [
            (
                "parent",
                MemberRole::Coordinator,
                MemberState::LIVE,
                vec![],
                None
            ),
            (
                "worker",
                MemberRole::Worker,
                MemberState::LIVE,
                vec![1],
                Some(42.5)
            )
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
    let before = read.revision().unwrap().revision;
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
    assert!(read.revision().unwrap().revision > before);
}

/// A member as the main projection test reads it.
type MemberSeen<'a> = (&'a str, MemberRole, MemberState, Vec<i64>, Option<f64>);

/// One change to a valid board.
type Edit = fn(&mut BoardState);

/// Each edit refused as an edited record, naming the record it found.
#[test]
fn values_the_board_never_writes_are_refused() {
    let edits: Vec<(&str, Edit)> = vec![
        ("task status", |state| {
            state.tasks[0].columns[4].1 = json!("verified")
        }),
        ("task status", |state| {
            state.tasks[0].columns[4].1 = json!(3)
        }),
        ("task title", |state| {
            state.tasks[0].columns[1].1 = json!(["x"])
        }),
        ("task acceptance", |state| {
            state.tasks[0].columns[2].1 = json!("pass")
        }),
        ("task dependencies", |state| {
            state.tasks[1].columns[3].1 = json!(["one"])
        }),
        ("evidence", |state| {
            state.evidence[0].1.accepted = json!("yes")
        }),
        ("run status", |state| {
            state.run.as_mut().unwrap().record.status = Some(RunState::new("verified"))
        }),
        ("member status", |state| {
            state.members[1].columns[2].1 = json!("zombie")
        }),
    ];
    for (record, edit) in edits {
        let mut state = board();
        edit(&mut state);
        let refused = ReadBoardProjection::new(MemoryBoard::with(state))
            .execute()
            .unwrap_err();
        assert_eq!(
            (refused.kind(), refused.message()),
            (
                RefusalKind::Store,
                format!("the board's {record} is not as the board writes it").as_str()
            ),
            "{record}"
        );
    }
}

/// Review round 1 (#2569): no budget is applied (owner decision,
/// 2026-10-14), so a stored budget no board writes never refuses the
/// projection: each setting it holds as written is reported, any other as
/// unset.
#[test]
fn a_budget_no_board_writes_is_reported_as_far_as_it_reads() {
    for (stored, want) in [
        (json!({"token_limit": "many"}), (None, false, false)),
        (
            json!({"token_limit": 1000, "strict_unknown": true}),
            (Some(1000), true, false),
        ),
        (
            json!({"token_limit": 1000, "warned": false}),
            (Some(1000), false, false),
        ),
        (
            json!({"strict_unknown": true, "warned": true}),
            (None, true, true),
        ),
        (
            json!({"token_limit": -1, "strict_unknown": "yes"}),
            (None, false, false),
        ),
        (json!([]), (None, false, false)),
        (json!(null), (None, false, false)),
    ] {
        let mut state = board();
        state.usage.as_mut().unwrap().budget = stored.clone();
        let budget = ReadBoardProjection::new(MemoryBoard::with(state))
            .execute()
            .unwrap()
            .run
            .unwrap()
            .budget;
        assert_eq!(
            (budget.token_limit, budget.strict_unknown, budget.warned),
            want,
            "{stored}"
        );
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
    let more = PROJECTION_LIMITS.members + 1;
    for index in 0..more {
        state.members.push(stored_member(
            &format!("extra-{index:03}"),
            json!(null),
            "dead",
            [json!(null), json!(null), json!(null), json!(null)],
        ));
    }
    let projection = project(state);
    assert_eq!(projection.members.len() as i64, PROJECTION_LIMITS.members);
    assert_eq!(projection.totals.members, more + 2);
    assert!(projection.truncated);
}

/// M2 (#2487 round 2): the board never holds more items than it lets be
/// created, so every item's state is read and one past that is an edited
/// board, refused rather than read without bound.
#[test]
fn a_board_holding_more_items_than_the_board_writes_is_refused() {
    let mut state = board();
    let more = PROJECTION_LIMITS.items + 1;
    state.tasks = (1..=more)
        .map(|id| stored_task(id, "ready", json!([]), None))
        .collect();
    let refused = ReadBoardProjection::new(MemoryBoard::with(state))
        .execute()
        .unwrap_err();
    assert_eq!(
        (refused.kind(), refused.message()),
        (
            RefusalKind::Store,
            "the board's tasks is not as the board writes it"
        )
    );
}

/// M3 (#2487 round 2): a member's claims are read from every item, so
/// they agree with the totals when its claimed item is left out.
#[test]
fn a_member_s_claims_come_from_every_item_not_only_those_that_fit() {
    let mut state = board();
    state.tasks[0].columns[1].1 = json!("x".repeat(10_000));
    let projection = ReadBoardProjection::new(MemoryBoard::with(state))
        .within(5_000)
        .unwrap();
    assert!(projection.items.iter().all(|item| item.id != 1));
    assert!(projection.truncated);
    let worker = projection
        .members
        .iter()
        .find(|member| member.id == "worker")
        .expect("the worker fits");
    assert_eq!(worker.claims, [1]);
    assert_eq!(projection.totals.claimed, 1);
}

/// M2 (#2487 round 2): the run's text counts against the budget, and a
/// run larger than the budget itself is an edited board (the board bounds
/// every run text far below it).
#[test]
fn the_run_s_text_counts_against_the_budget() {
    let mut state = board();
    state.run.as_mut().unwrap().record.outcome_reason = Some("x".repeat(4_000));
    let projection = ReadBoardProjection::new(MemoryBoard::with(state.clone()))
        .within(5_000)
        .unwrap();
    assert!(projection.truncated);
    assert!(projection.items.len() < 4, "{:?}", projection.items);
    state.run.as_mut().unwrap().record.outcome_reason = Some("x".repeat(6_000));
    let refused = ReadBoardProjection::new(MemoryBoard::with(state))
        .within(5_000)
        .unwrap_err();
    assert_eq!(
        (refused.kind(), refused.message()),
        (
            RefusalKind::Store,
            "the board's run is not as the board writes it"
        )
    );
}

/// L4 (#2487 round 2): the revision names the run whose board it counts,
/// so a recreated board at the same revision is still told apart.
#[test]
fn the_revision_names_its_run() {
    let read = ReadBoardProjection::new(MemoryBoard::with(board()));
    let revision = read.revision().unwrap();
    assert_eq!(revision.run.as_deref(), Some("run-1"));
    assert!(revision.created, "round 3 L3: a created run's deadline");
    assert_eq!(read.execute().unwrap().revision, revision.revision);
    let empty = ReadBoardProjection::new(MemoryBoard::with(BoardState::default()))
        .revision()
        .unwrap();
    assert_eq!((empty.run, empty.revision, empty.created), (None, 0, false));
}

/// L5 (#2487 round 2): a listed member holds one of the three roles; any
/// other answer is the harness's own error, never a guessed worker.
#[test]
fn a_role_outside_the_three_a_member_holds_is_an_internal_error() {
    assert_eq!(
        super::member_role(Some(BoardRole::Worker)).unwrap(),
        MemberRole::Worker
    );
    for role in [Some(BoardRole::Host), None] {
        assert_eq!(
            super::member_role(role).unwrap_err().kind(),
            RefusalKind::Internal
        );
    }
}

/// M1 (#2487 round 2): the budget's measure of text and JSON values is
/// their encoded length, escapes included.
#[test]
fn text_and_values_are_measured_as_encoded() {
    for text in [
        "",
        "plain",
        "quote \" and \\ backslash",
        "\u{1}\u{8}\u{c}\n\r\t\u{1f}\u{7f}",
        "é 中 😀",
    ] {
        assert_eq!(
            super::encoded_text(text),
            json!(text).to_string().len(),
            "{text:?}"
        );
    }
    let value = json!({"a": [1, -2.5, null, true, false, "\u{1}"], "\n": {"x": "y"}});
    assert_eq!(super::encoded_value(&value), value.to_string().len());
}
