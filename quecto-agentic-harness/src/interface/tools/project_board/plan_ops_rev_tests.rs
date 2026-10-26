//! #2705 B9: a read is pinned by the board's revision, `rev`, which
//! `show` answers; `expected_updated` is taken for one more release, its
//! answer saying it is deprecated.
use serde_json::json;

use super::plan_ops_tests::{call, drafted, item, ok};

/// The note an answer to a request pinned by `expected_updated` carries.
const DEPRECATED: &str = "expected_updated is deprecated and will be removed: it no longer \
     tells two edits in one second apart, so it may overwrite a newer edit; pass expected_rev, \
     the rev show gives";

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

/// #2705 B9 review round 1: a rev pin is judged where the stamp pin was,
/// after the draft and state checks, so a retried acceptance whose first
/// try landed is told the plan is no draft (`plan_not_draft`), as a
/// retry of a discard is, and an edit of a draft accepted since.
#[test]
fn a_retry_after_its_first_try_landed_is_told_the_plan_is_no_draft() {
    let (_store, ada, _bob) = drafted();
    let read = ok(&ada, json!({"op": "show", "task": "a", "version": 2}));
    let accept = json!({"op": "accept_plan", "task": "a", "version": 2,
                        "expected_rev": read["rev"], "expected_accepted": null});
    ok(&ada, accept.clone());
    let retried = call(&ada, accept);
    assert_eq!(
        (
            &retried["error"]["rule"]["code"],
            &retried["error"]["rule"]["state"]
        ),
        (&json!("plan_not_draft"), &json!("accepted")),
        "{retried}"
    );
    let edited = call(
        &ada,
        json!({"op": "edit_plan", "task": "a", "version": 2, "items": [item("late")],
               "expected_rev": read["rev"]}),
    );
    assert_eq!(
        edited["error"]["rule"]["code"],
        json!("plan_not_draft"),
        "{edited}"
    );
    let read = ok(&ada, json!({"op": "show", "task": "a", "version": 1}));
    let discard = json!({"op": "discard_plan", "task": "a", "version": 1,
                         "expected_rev": read["rev"]});
    ok(&ada, discard.clone());
    let retried = call(&ada, discard);
    assert_eq!(
        (
            &retried["error"]["rule"]["code"],
            &retried["error"]["rule"]["state"]
        ),
        (&json!("plan_not_draft"), &json!("discarded")),
        "{retried}"
    );
}

/// #2705 B9 review round 1: a page pinned to the rev of the first page
/// holds while that plan is as it was, whatever else the board took, and
/// is refused (`plan_changed`) once the plan changed: read again from 0.
#[test]
fn a_page_pinned_by_rev_restarts_only_when_its_plan_changed() {
    let (_store, ada, bob) = drafted();
    let first = ok(
        &bob,
        json!({"op": "show", "task": "a", "version": 2, "approach_from": 0}),
    );
    ok(&ada, json!({"op": "create", "task": "b", "title": "B"}));
    let next = ok(
        &bob,
        json!({"op": "show", "task": "a", "version": 2, "items_from": 0,
               "expected_rev": first["rev"]}),
    );
    assert_ne!(next["rev"], first["rev"], "the board moved");
    assert_eq!(next["plan"]["items"], json!([item("schema"), item("api")]));
    ok(
        &ada,
        json!({"op": "edit_plan", "task": "a", "version": 2, "items": [item("other")]}),
    );
    let stale = call(
        &bob,
        json!({"op": "show", "task": "a", "version": 2, "items_from": 0,
               "expected_rev": first["rev"]}),
    );
    assert_eq!(
        stale["error"]["rule"]["code"],
        json!("plan_changed"),
        "{stale}"
    );
    let unpaged = call(
        &bob,
        json!({"op": "show", "task": "a", "version": 2, "expected_rev": first["rev"]}),
    );
    assert_eq!(
        (&unpaged["error"]["code"], &unpaged["error"]["field"]),
        (&json!("invalid_request"), &json!("expected_rev")),
        "a pin is a page's: {unpaged}"
    );
}
