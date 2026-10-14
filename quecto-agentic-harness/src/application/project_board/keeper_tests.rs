use serde_json::{Value, json};

use super::*;

/// A projection as the live view answers it: a run of `status` (none for
/// `null`), the coordinator member's `coordinator` status, and `items`.
pub fn projection(status: Option<&str>, coordinator: &str, items: Value) -> Value {
    let run = status.map_or(Value::Null, |status| {
        json!({"id": "r1", "goal": "Board task board-store, plan v1: Board store",
               "status": status, "budget": {"observed_tokens": 52_000}})
    });
    json!({
        "revision": 7,
        "run": run,
        "items": items,
        "members": [
            {"id": "worker-1", "role": "worker", "status": "live"},
            {"id": "lead", "role": "coordinator", "status": coordinator},
        ],
        "evidence": [],
        "truncated": false,
    })
}

/// A still-going swarm (paused included) is going, with whether its
/// coordinator is a live member; a board before its run is going too.
#[test]
fn a_swarm_still_going_shows_whether_its_coordinator_lives() {
    for status in [Some("running"), Some("paused"), Some("setup"), None] {
        for (coordinator, alive) in [("live", true), ("reserved", true), ("dead", false)] {
            assert_eq!(
                sight_of(&projection(status, coordinator, json!([]))),
                Some(SwarmSight::Going {
                    coordinator_alive: alive
                }),
                "{status:?} {coordinator}"
            );
        }
    }
}

/// An ended swarm is seen as its view: its end, its items with their
/// states, owners and evidence (entries that are not an artifact and a
/// revision skipped, items of a state the swarm never writes left out),
/// and its observed tokens.
#[test]
fn an_ended_swarm_is_seen_as_its_run_view() {
    let items = json!([
        {"title": "api: Build the api", "state": "completed", "owner": "worker-1",
         "evidence": [{"artifact": "src/api.rs", "revision": "4f2a9c1"},
                      "loose text", {"artifact": "no revision"}]},
        {"title": "store: Build the store", "state": "claimed", "owner": null,
         "evidence": null},
        {"title": "odd", "state": "archived"},
        {"title": 7, "state": "ready"},
    ]);
    let seen = sight_of(&projection(Some("budget-exhausted"), "dead", items));
    let Some(SwarmSight::Ended(view)) = seen else {
        panic!("not ended: {seen:?}");
    };
    assert_eq!(view.end, SwarmEnd::BudgetExhausted);
    assert_eq!(view.observed_tokens, 52_000);
    assert_eq!(
        view.items,
        vec![
            SwarmItemView {
                title: "api: Build the api".into(),
                state: SwarmItemState::Completed,
                owner: Some("worker-1".into()),
                evidence: vec![SwarmEvidence {
                    artifact: "src/api.rs".into(),
                    revision: "4f2a9c1".into(),
                }],
            },
            SwarmItemView {
                title: "store: Build the store".into(),
                state: SwarmItemState::Claimed,
                owner: None,
                evidence: vec![],
            },
        ]
    );
    for status in ["succeeded", "failed", "blocked", "cancelled"] {
        assert!(
            matches!(
                sight_of(&projection(Some(status), "live", json!([]))),
                Some(SwarmSight::Ended(_))
            ),
            "{status}"
        );
    }
}

/// Spent tokens below zero, or not a number, count as none.
#[test]
fn tokens_the_board_cannot_count_are_none() {
    let mut board = projection(Some("succeeded"), "live", json!([]));
    board["run"]["budget"]["observed_tokens"] = json!(-5);
    let Some(SwarmSight::Ended(view)) = sight_of(&board) else {
        panic!("ended");
    };
    assert_eq!(view.observed_tokens, 0);
}

/// A projection not of the live view's shape is unread.
#[test]
fn a_projection_of_another_shape_is_unread() {
    let mut no_members = projection(Some("running"), "live", json!([]));
    no_members.as_object_mut().unwrap().remove("members");
    let mut no_status = projection(Some("running"), "live", json!([]));
    no_status["run"]["status"] = json!(3);
    let mut no_items = projection(Some("succeeded"), "live", json!([]));
    no_items["items"] = json!({});
    for board in [json!([]), json!({}), no_members, no_status, no_items] {
        assert_eq!(sight_of(&board), None, "{board}");
    }
}
