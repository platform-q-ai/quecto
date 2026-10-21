use serde_json::{Value, json};

use super::*;

/// A board's projection as the dispatch host answers `_board`: run
/// `status`, `observed` tokens spent, and `items`.
pub fn board(status: &str, observed: u64, items: Value) -> Value {
    let total = items.as_array().map_or(0, Vec::len);
    json!({
        "revision": 41,
        "run": {
            "id": "run-1",
            "status": status,
            "outcome": null,
            "budget": {"token_limit": null, "observed_tokens": observed},
        },
        "items": items,
        "members": [],
        "evidence": [],
        "totals": {"items": total},
        "truncated": false,
    })
}

/// A succeeded swarm's board reads as its end, items and spend.
#[test]
fn an_ended_boards_items_owners_evidence_and_spend_are_read() {
    let board = board(
        "succeeded",
        52_000,
        json!([
            {
                "id": 1,
                "title": "assess-failure: Assess the failure",
                "state": "completed",
                "owner": "worker-1",
                "evidence": [
                    {"artifact": "docs/assessment.md", "revision": "abc123"},
                    {"artifact": 7, "revision": "abc123"},
                    {"artifact": "docs/x.md"},
                ],
            },
            {"id": 2, "title": "uk-shopping-list: List", "state": "submitted", "owner": null, "evidence": []},
        ]),
    );
    let view = swarm_run_view(&board).unwrap();
    assert_eq!(view.end, SwarmEnd::Succeeded);
    assert_eq!(view.observed_tokens, 52_000);
    assert_eq!(
        view.items,
        vec![
            SwarmItemView {
                title: "assess-failure: Assess the failure".into(),
                state: SwarmItemState::Completed,
                owner: Some("worker-1".into()),
                evidence: vec![SwarmEvidence {
                    artifact: "docs/assessment.md".into(),
                    revision: "abc123".into(),
                }],
                death_blocked: false,
            },
            SwarmItemView {
                title: "uk-shopping-list: List".into(),
                state: SwarmItemState::Submitted,
                owner: None,
                evidence: vec![],
                death_blocked: false,
            },
        ],
        "an evidence entry whose parts are not both text is left out"
    );
}

/// Each status that ends a swarm reads as its end; any other is no view.
#[test]
fn only_an_ended_run_is_a_view() {
    for (status, end) in [
        ("succeeded", SwarmEnd::Succeeded),
        ("failed", SwarmEnd::Failed),
        ("budget-exhausted", SwarmEnd::BudgetExhausted),
        ("blocked", SwarmEnd::Blocked),
        ("cancelled", SwarmEnd::Cancelled),
    ] {
        let view = swarm_run_view(&board(status, 1, json!([]))).unwrap();
        assert_eq!(view.end, end, "{status}");
    }
    for status in ["running", "paused", "setting-up", "", "Succeeded"] {
        assert_eq!(
            swarm_run_view(&board(status, 1, json!([]))),
            None,
            "{status}"
        );
    }
}

/// A board with no run, no readable spend or no items is no view; nor is
/// one that left items out, or does not count them. One that left only
/// other rows out (members, evidence) is read whole.
#[test]
fn a_board_missing_its_run_spend_or_items_or_left_short_is_no_view() {
    let item = json!({"title": "a: A", "state": "completed"});
    let mut short = board("succeeded", 1, json!([item.clone()]));
    short["totals"]["items"] = json!(2);
    let mut uncounted = board("succeeded", 1, json!([]));
    uncounted.as_object_mut().unwrap().remove("totals");
    assert_eq!(swarm_run_view(&short), None, "an item left out");
    assert_eq!(swarm_run_view(&uncounted), None, "not counted");
    let mut members_cut = board("succeeded", 1, json!([item]));
    members_cut["truncated"] = json!(true);
    assert_eq!(
        swarm_run_view(&members_cut).map(|view| view.items.len()),
        Some(1),
        "its items whole, other rows cut"
    );
    let mut no_run = board("succeeded", 1, json!([]));
    no_run["run"] = Value::Null;
    let mut no_spend = board("succeeded", 1, json!([]));
    no_spend["run"]["budget"]["observed_tokens"] = json!(-1);
    let mut no_items = board("succeeded", 1, json!([]));
    no_items["items"] = json!({});
    for board in [no_run, no_spend, no_items, json!([]), json!("board")] {
        assert_eq!(swarm_run_view(&board), None, "{board}");
    }
}

/// An item whose title is not text, or whose state the swarm does not
/// write, is left out.
#[test]
fn an_item_that_cannot_be_read_is_left_out() {
    let board = board(
        "succeeded",
        1,
        json!([
            {"title": 7, "state": "completed"},
            {"title": "a: A", "state": "done"},
            {"title": "b: B"},
            {"title": "c: C", "state": "completed", "owner": 9, "evidence": "x"},
        ]),
    );
    let view = swarm_run_view(&board).unwrap();
    assert_eq!(
        view.items,
        vec![SwarmItemView {
            title: "c: C".into(),
            state: SwarmItemState::Completed,
            owner: None,
            evidence: vec![],
            death_blocked: false,
        }]
    );
}

/// #2685: a board is kept once its coordinator's teardown has ended its
/// workers, and each worker's death blocked what it held ("worker death
/// confirmed …"). The item is read as the board holds it (blocked, so a
/// duplicate is ranked by its own state) and marked blocked by a death;
/// the fold-back reads it as where it stood, in review when it holds
/// evidence, else doing. A blocker of any other text is the item's own,
/// and stays blocked, evidence or none. Ended and left boards alike.
#[test]
fn a_teardown_death_blocker_reads_as_where_the_item_stood() {
    use crate::domain::project_board::value_objects::vocabulary::ItemState;
    let items = json!([
        {
            "id": 1, "title": "implement: Implement", "state": "blocked", "owner": "worker-1",
            "blocker": "worker death confirmed; coordinator recovery required",
            "evidence": [{"artifact": "src/lib.rs", "revision": "abc123"}],
        },
        {
            "id": 2, "title": "test: Test", "state": "blocked", "owner": "worker-2",
            "blocker": "worker death confirmed (abrupt exit; reservations retained); coordinator recovery required",
            "evidence": [],
        },
        {
            "id": 3, "title": "docs: Docs", "state": "blocked", "owner": "worker-3",
            "blocker": "waiting for the API key", "evidence": [],
        },
        {
            "id": 4, "title": "lint: Lint", "state": "blocked", "owner": "worker-4",
            "blocker": "waiting for review access",
            "evidence": [{"artifact": "src/lint.rs", "revision": "def456"}],
        },
    ]);
    let read = |view: SwarmRunView| -> Vec<(SwarmItemState, bool, ItemState)> {
        view.items
            .iter()
            .map(|item| (item.state, item.death_blocked, item.item_state()))
            .collect()
    };
    let expected = vec![
        (SwarmItemState::Blocked, true, ItemState::Review),
        (SwarmItemState::Blocked, true, ItemState::Doing),
        (SwarmItemState::Blocked, false, ItemState::Blocked),
        (SwarmItemState::Blocked, false, ItemState::Blocked),
    ];
    assert_eq!(
        read(left_run_view(&board("running", 10, items.clone())).unwrap()),
        expected
    );
    assert_eq!(
        read(swarm_run_view(&board("cancelled", 10, items)).unwrap()),
        expected
    );
}
