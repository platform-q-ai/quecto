//! #2705 B9: a read is pinned by the board's revision, `rev`, which
//! `show` answers; `expected_updated` is taken for one more release, its
//! answer saying it is deprecated.
use serde_json::json;

use super::plan_ops_tests::{call, drafted, item, ok};

/// The note an answer to a request pinned by `expected_updated` carries.
const DEPRECATED: &str =
    "expected_updated is deprecated and will be removed: pass expected_rev, the rev show gives";

#[test]
fn show_and_its_pages_answer_the_rev_they_were_read_at() {
    let (_store, ada, _bob) = drafted();
    let shown = ok(&ada, json!({"op": "show", "task": "a", "version": 2}));
    assert!(shown["rev"].is_string(), "{shown}");
    assert_eq!(shown["rev"], shown["head"]);
    let page = ok(
        &ada,
        json!({"op": "show", "task": "a", "version": 2, "items_from": 0}),
    );
    assert_eq!(page["rev"], shown["rev"], "{page}");
}

#[test]
fn a_stale_expected_rev_gives_plan_changed() {
    let (store, ada, bob) = drafted();
    let read = ok(&bob, json!({"op": "show", "task": "a", "version": 2}));
    ok(
        &ada,
        json!({"op": "edit_plan", "task": "a", "version": 2, "items": [item("ada")],
               "expected_rev": read["rev"]}),
    );
    let writes = store.writes();
    let late = call(
        &bob,
        json!({"op": "edit_plan", "task": "a", "version": 2, "items": [item("bob")],
               "expected_rev": read["rev"]}),
    );
    assert_eq!(
        late["error"]["rule"]["code"],
        json!("plan_changed"),
        "{late}"
    );
    assert_eq!(store.writes(), writes, "nothing written");
    let accept = call(
        &bob,
        json!({"op": "accept_plan", "task": "a", "version": 2,
               "expected_rev": read["rev"], "expected_accepted": null}),
    );
    assert_eq!(
        accept["error"]["rule"]["code"],
        json!("plan_changed"),
        "{accept}"
    );
    assert!(accept.get("deprecated").is_none(), "{accept}");
}

/// A pin holds while the artifact it pins is as it was read, whatever
/// else the board took since.
#[test]
fn a_change_elsewhere_on_the_board_leaves_a_rev_pin_holding() {
    let (_store, ada, bob) = drafted();
    let read = ok(&bob, json!({"op": "show", "task": "a", "version": 2}));
    ok(&ada, json!({"op": "create", "task": "b", "title": "B"}));
    ok(
        &ada,
        json!({"op": "edit_plan", "task": "a", "version": 1, "items": [item("one")]}),
    );
    let edited = ok(
        &bob,
        json!({"op": "edit_plan", "task": "a", "version": 2, "items": [item("bob")],
               "expected_rev": read["rev"]}),
    );
    assert!(edited.get("deprecated").is_none(), "{edited}");
    let shown = ok(&bob, json!({"op": "show", "task": "a", "version": 2}));
    let accepted = ok(
        &bob,
        json!({"op": "accept_plan", "task": "a", "version": 2,
               "expected_rev": shown["rev"], "expected_accepted": null}),
    );
    assert_eq!(accepted["plan"]["state"], json!("accepted"), "{accepted}");
}

#[test]
fn a_legacy_expected_updated_still_works_and_its_answer_says_it_is_deprecated() {
    let (_store, ada, bob) = drafted();
    let read = ok(&bob, json!({"op": "show", "task": "a", "version": 2}));
    let edited = ok(
        &bob,
        json!({"op": "edit_plan", "task": "a", "version": 2, "items": [item("bob")],
               "expected_updated": read["plan"]["updated"]}),
    );
    assert_eq!(edited["deprecated"], json!(DEPRECATED), "{edited}");
    let read = ok(&ada, json!({"op": "show", "task": "a", "version": 2}));
    let accepted = ok(
        &ada,
        json!({"op": "accept_plan", "task": "a", "version": 2,
               "expected_updated": read["plan"]["updated"], "expected_accepted": null}),
    );
    assert_eq!(accepted["deprecated"], json!(DEPRECATED), "{accepted}");
    let task = ok(&ada, json!({"op": "create", "task": "d", "title": "D"}));
    let updated = ok(
        &ada,
        json!({"op": "update_draft", "task": "d", "title": "D2",
               "expected_updated": task["task"]["updated"]}),
    );
    assert_eq!(updated["deprecated"], json!(DEPRECATED), "{updated}");
}

#[test]
fn update_draft_is_pinned_by_rev_and_a_stale_one_is_stale() {
    let (store, ada, bob) = drafted();
    let read = ok(&bob, json!({"op": "show", "task": "a"}));
    ok(
        &ada,
        json!({"op": "update_draft", "task": "a", "title": "Ada's",
               "expected_rev": read["rev"]}),
    );
    let writes = store.writes();
    let late = call(
        &bob,
        json!({"op": "update_draft", "task": "a", "title": "Bob's",
               "expected_rev": read["rev"]}),
    );
    assert_eq!(
        (&late["error"]["code"], &late["error"]["expected"]),
        (&json!("stale"), &read["rev"]),
        "{late}"
    );
    assert_eq!(store.writes(), writes, "nothing written");
}

#[test]
fn a_pin_is_given_once_and_an_acceptance_needs_one() {
    let (_store, ada, _bob) = drafted();
    let read = ok(&ada, json!({"op": "show", "task": "a", "version": 2}));
    let both = call(
        &ada,
        json!({"op": "edit_plan", "task": "a", "version": 2, "items": [],
               "expected_rev": read["rev"], "expected_updated": read["plan"]["updated"]}),
    );
    assert_eq!(
        (&both["error"]["code"], &both["error"]["field"]),
        (&json!("invalid_request"), &json!("expected_updated")),
        "{both}"
    );
    let neither = call(
        &ada,
        json!({"op": "accept_plan", "task": "a", "version": 2, "expected_accepted": null}),
    );
    assert_eq!(
        (&neither["error"]["code"], &neither["error"]["field"]),
        (&json!("invalid_request"), &json!("expected_rev")),
        "{neither}"
    );
    let unspellable = call(
        &ada,
        json!({"op": "edit_plan", "task": "a", "version": 2, "items": [],
               "expected_rev": "not a rev"}),
    );
    assert_eq!(
        (
            &unspellable["error"]["code"],
            &unspellable["error"]["field"]
        ),
        (&json!("invalid_request"), &json!("expected_rev")),
        "{unspellable}"
    );
}
