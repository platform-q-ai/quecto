use serde_json::{Value, json};

use super::*;

/// A board's projection as the dispatch host answers `_board`: run
/// `status`, `observed` tokens spent, and `items`.
pub fn board(status: &str, observed: u64, items: Value) -> Value {
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
            },
            SwarmItemView {
                title: "uk-shopping-list: List".into(),
                state: SwarmItemState::Submitted,
                owner: None,
                evidence: vec![],
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
/// one that left rows out, or does not say whether it did.
#[test]
fn a_board_missing_its_run_spend_or_items_or_left_short_is_no_view() {
    let mut truncated = board("succeeded", 1, json!([]));
    truncated["truncated"] = json!(true);
    let mut unsaid = board("succeeded", 1, json!([]));
    unsaid.as_object_mut().unwrap().remove("truncated");
    assert_eq!(swarm_run_view(&truncated), None, "rows left out");
    assert_eq!(swarm_run_view(&unsaid), None, "not saying");
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
        }]
    );
}
