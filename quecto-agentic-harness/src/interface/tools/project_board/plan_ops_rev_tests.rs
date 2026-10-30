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
/// Approved parent contract: exercise the real JSON dispatch without naming
/// production symbols that do not exist yet. Writer and clock are trusted
/// inputs from the existing assembly, never request fields.
fn done_ticks_finished_board() -> (
    crate::infrastructure::test_support::memory_board_store::MemoryBoardStore,
    crate::composition::bootstrap::project_board_operations::LocalBoardOperations,
) {
    let store =
        crate::infrastructure::test_support::memory_board_store::MemoryBoardStore::default();
    use crate::application::project_board::ports::ProjectBoardStore;
    store.bootstrap().unwrap();
    let writer = super::plan_ops_tests::as_writer(&store, "Ada");
    ok(
        &writer,
        json!({"op":"create","task":"a","title":"Ship safely","status":"ready",
        "goal":"Ship safely.","done_when":["Same outcome","Same outcome"]}),
    );
    ok(
        &writer,
        json!({"op":"done_without_run","task":"a","reason":"Answered by hand"}),
    );
    (store, writer)
}
fn done_ticks_request(rev: &serde_json::Value) -> serde_json::Value {
    json!({"op":"set_outcome","task":"a","outcome_index":0,"met":false,
        "reason":"Owner deferred deployment","expected_rev":rev})
}

#[test]
fn done_ticks_exception_is_attributed_roundtrips_answers_and_restores_without_reopening() {
    use crate::application::project_board::ports::ProjectBoardStore;
    let (store, writer) = done_ticks_finished_board();
    let before = ok(&writer, json!({"op":"show","task":"a"}));
    let mut request = done_ticks_request(&before["rev"]);
    request["reason"] = json!("  Owner\u{a0}deferred deployment  ");
    let changed = ok(&writer, request);
    let task = &changed["task"];
    assert_eq!(task["status"], "done");
    assert!(task.get("reason").is_none());
    assert_eq!(task["done_when"][0]["outcome"], "Same outcome");
    assert_eq!(task["done_when"][0]["done"], false);
    assert_eq!(
        task["done_when"][1]["done"], true,
        "index, not duplicate outcome text"
    );
    assert_eq!(
        task["done_when"][0]["exception"]["reason"],
        "Owner deferred deployment"
    );
    assert_eq!(
        task["done_when"][0]["exception"]["by"],
        json!({"name":"Ada","email":"ada@example.com"})
    );
    assert_eq!(task["done_when"][0]["exception"]["at"], task["updated"]);
    assert!(task["done_when"][1].get("exception").is_none());
    let description = task["description"].as_str().unwrap();
    assert!(
        description.contains("## Outcome exceptions"),
        "{description}"
    );
    assert!(
        description.contains("Owner deferred deployment"),
        "{description}"
    );
    let shown = ok(&writer, json!({"op":"show","task":"a"}));
    assert_eq!(
        shown["task"], *task,
        "fresh reads preserve exception answers"
    );
    let listed = ok(&writer, json!({"op":"list"}));
    assert_eq!(listed["tasks"][0]["done_when"], task["done_when"]);
    let restored = ok(
        &writer,
        json!({"op":"set_outcome","task":"a","outcome_index":0,
        "met":true,"expected_rev":shown["rev"]}),
    );
    assert_eq!(restored["task"]["status"], "done");
    assert!(restored["task"].get("reason").is_none());
    assert_eq!(restored["task"]["done_when"][0]["done"], true);
    assert!(restored["task"]["done_when"][0].get("exception").is_none());
    assert!(
        !restored["task"]["description"]
            .as_str()
            .unwrap()
            .contains("## Outcome exceptions")
    );
    assert_eq!(
        store.read().unwrap().tasks[0].fields().status,
        crate::domain::project_board::value_objects::vocabulary::TaskStatus::Done
    );
}

#[test]
fn done_ticks_exception_survives_archiving_and_subsequent_reads() {
    let (_store, writer) = done_ticks_finished_board();
    let before = ok(&writer, json!({"op":"show","task":"a"}));
    let changed = ok(&writer, done_ticks_request(&before["rev"]));
    let archived = ok(&writer, json!({"op":"move","task":"a","to":"archived"}));
    assert_eq!(archived["task"]["status"], "archived");
    assert_eq!(archived["task"]["done_when"], changed["task"]["done_when"]);
    assert!(archived["task"].get("reason").is_none());
    let shown = ok(&writer, json!({"op":"show","task":"a"}));
    assert_eq!(shown["task"], archived["task"]);
}

#[test]
fn done_ticks_exception_rejects_stale_and_unknown_board_revisions_without_writes() {
    use crate::application::project_board::ports::ProjectBoardStore;
    let (store, writer) = done_ticks_finished_board();
    let read = ok(&writer, json!({"op":"show","task":"a"}));
    ok(&writer, done_ticks_request(&read["rev"]));
    for rev in [
        read["rev"].clone(),
        json!("ffffffffffffffffffffffffffffffffffffffff"),
    ] {
        let before = store.read().unwrap();
        let writes = store.writes();
        let refused = call(
            &writer,
            json!({"op":"set_outcome","task":"a","outcome_index":0,
            "met":true,"expected_rev":rev}),
        );
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["error"]["code"], "stale", "{refused}");
        assert_eq!(store.read().unwrap().tasks, before.tasks);
        assert_eq!(store.writes(), writes);
    }
}

#[test]
fn done_ticks_exception_validates_reference_reason_types_and_required_rev_atomically() {
    use crate::application::project_board::ports::ProjectBoardStore;
    let (store, writer) = done_ticks_finished_board();
    let read = ok(&writer, json!({"op":"show","task":"a"}));
    let valid = done_ticks_request(&read["rev"]);
    let mut cases = vec![];
    for (field, value) in [
        ("outcome_index", json!(-1)),
        ("outcome_index", json!(0.5)),
        ("outcome_index", json!("0")),
        ("outcome_index", json!(2)),
        ("outcome_index", json!(u64::MAX)),
        ("met", json!("false")),
        ("met", json!(null)),
        ("reason", json!(" \t\u{a0}")),
        ("reason", json!("")),
        ("reason", json!(null)),
        ("reason", json!("a".repeat(1001))),
        ("expected_rev", json!(null)),
        ("expected_rev", json!("")),
        ("expected_rev", json!("BAD REV")),
    ] {
        let mut request = valid.clone();
        request[field] = value;
        cases.push((field, request));
    }
    for field in ["outcome_index", "met", "reason", "expected_rev"] {
        let mut request = valid.clone();
        request.as_object_mut().unwrap().remove(field);
        cases.push((field, request));
    }
    let mut reason_on_met = valid.clone();
    reason_on_met["met"] = json!(true);
    cases.push(("reason", reason_on_met));
    let mut deprecated = valid.clone();
    deprecated.as_object_mut().unwrap().remove("expected_rev");
    deprecated["expected_updated"] = read["task"]["updated"].clone();
    cases.push(("expected_updated", deprecated));
    for (field, request) in cases {
        let before = store.read().unwrap();
        let writes = store.writes();
        let answer = call(&writer, request.clone());
        assert_eq!(answer["ok"], false, "{request}: {answer}");
        // Do not let the baseline's unknown-op refusal masquerade as a
        // passing field-validation test.
        let error_field = answer["error"]["field"]
            .as_str()
            .or_else(|| answer["error"]["rule"]["field"].as_str());
        assert_eq!(error_field, Some(field), "{request}: {answer}");
        assert_eq!(store.read().unwrap().tasks, before.tasks);
        assert_eq!(store.writes(), writes);
    }
}

#[test]
fn done_ticks_exception_uses_done_only_allowlist_and_cannot_spoof_attribution() {
    use crate::application::project_board::ports::ProjectBoardStore;
    use crate::domain::project_board::entities::task::Task;
    use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
    for status in [
        TaskStatus::Draft,
        TaskStatus::Ready,
        TaskStatus::Claimed,
        TaskStatus::InProgress,
        TaskStatus::Review,
        TaskStatus::Blocked,
        TaskStatus::Archived,
    ] {
        let (store, writer) = done_ticks_finished_board();
        let board = store.read().unwrap();
        let mut fields = board.tasks[0].fields().clone();
        fields.status = status;
        fields.reason = None;
        fields.resolution = None;
        if matches!(status, TaskStatus::Claimed | TaskStatus::InProgress) {
            let at = fields.updated.clone();
            fields.claim = Some(crate::domain::project_board::entities::claim::Claim {
                holder: crate::domain::project_board::entities::claim::Identity {
                    name: "Ada".into(),
                    email: "ada@example.com".into(),
                },
                since: at.clone(),
                expires: at.plus_seconds(3600).unwrap(),
            });
        }
        store
            .write_tasks(&board.head, &[Task::new(fields).unwrap()], "status fixture")
            .unwrap();
        let read = ok(&writer, json!({"op":"show","task":"a"}));
        let before = store.read().unwrap();
        let writes = store.writes();
        let refused = call(&writer, done_ticks_request(&read["rev"]));
        assert_eq!(refused["ok"], false, "{status:?}: {refused}");
        assert_eq!(refused["error"]["code"], "rule", "{status:?}: {refused}");
        assert_eq!(store.read().unwrap().tasks, before.tasks);
        assert_eq!(store.writes(), writes);
    }
    let (store, writer) = done_ticks_finished_board();
    let read = ok(&writer, json!({"op":"show","task":"a"}));
    for (field, value) in [
        ("by", json!({"name":"Mallory","email":"m@example.com"})),
        ("at", json!("2000-01-01T00:00:00Z")),
    ] {
        let mut request = done_ticks_request(&read["rev"]);
        request[field] = value;
        let writes = store.writes();
        let refused = call(&writer, request);
        assert_eq!(refused["error"]["field"], field, "{refused}");
        assert_eq!(store.writes(), writes);
    }
}

#[test]
fn done_ticks_existing_cli_envelope_builds_the_same_exception_request() {
    let flags = [
        "--task",
        "a",
        "--outcome-index",
        "0",
        "--met",
        "false",
        "--reason",
        "Owner deferred deployment",
        "--expected-rev",
        "r3",
    ]
    .map(String::from);
    let request = super::command_line::request("set-outcome", &flags)
        .expect("approved board operation through existing CLI envelope");
    assert_eq!(request, done_ticks_request(&json!("r3")));
}

#[test]
fn done_ticks_exception_task_pin_survives_an_unrelated_write() {
    let (_store, writer) = done_ticks_finished_board();
    let read = ok(&writer, json!({"op":"show","task":"a"}));
    ok(
        &writer,
        json!({"op":"create","task":"unrelated","title":"Another task"}),
    );
    let changed = ok(&writer, done_ticks_request(&read["rev"]));
    assert_eq!(changed["task"]["done_when"][0]["done"], false);
}

#[test]
fn done_ticks_restore_refuses_any_reason_and_null_index_without_writes() {
    use crate::application::project_board::ports::ProjectBoardStore;
    let (store, writer) = done_ticks_finished_board();
    let read = ok(&writer, json!({"op":"show","task":"a"}));
    for reason in [json!(null), json!(""), json!(" ")] {
        let before = store.read().unwrap();
        let writes = store.writes();
        let answer = call(
            &writer,
            json!({"op":"set_outcome","task":"a","outcome_index":0,"met":true,"reason":reason,"expected_rev":read["rev"]}),
        );
        assert_eq!(answer["error"]["field"], "reason", "{answer}");
        assert_eq!(store.read().unwrap().tasks, before.tasks);
        assert_eq!(store.writes(), writes);
    }
    let mut request = done_ticks_request(&read["rev"]);
    request["outcome_index"] = json!(null);
    let writes = store.writes();
    let before = store.read().unwrap();
    let answer = call(&writer, request);
    assert_eq!(answer["error"]["field"], "outcome_index", "{answer}");
    assert_eq!(store.read().unwrap().tasks, before.tasks);
    assert_eq!(store.writes(), writes);
}

#[test]
fn done_ticks_exception_retry_keeps_task_pin_after_unrelated_competing_write() {
    use crate::application::project_board::ports::ProjectBoardStore;
    use crate::domain::project_board::entities::task::Task;
    let (store, writer) = done_ticks_finished_board();
    ok(
        &writer,
        json!({"op":"create","task":"unrelated","title":"Before"}),
    );
    let read = ok(&writer, json!({"op":"show","task":"a"}));
    let before = store.read().unwrap();
    let writes = store.writes();
    store.before_next_write(|store| {
        let snapshot = store.read().unwrap();
        let mut unrelated = snapshot
            .tasks
            .iter()
            .find(|task| task.fields().id.as_str() == "unrelated")
            .unwrap()
            .fields()
            .clone();
        unrelated.title = "Concurrent edit".into();
        store
            .write_tasks(
                &snapshot.head,
                &[Task::new(unrelated).unwrap()],
                "competing write",
            )
            .unwrap();
    });
    let answer = call(&writer, done_ticks_request(&read["rev"]));
    assert_eq!(answer["ok"], true, "{answer}");
    let after = store.read().unwrap();
    let unrelated = after
        .tasks
        .iter()
        .find(|task| task.fields().id.as_str() == "unrelated")
        .unwrap();
    assert_eq!(unrelated.fields().title, "Concurrent edit");
    assert_eq!(answer["task"]["done_when"][0]["done"], false);
    assert_eq!(
        store.writes(),
        writes + 2,
        "competing and requested writes both land"
    );
    assert_eq!(after.tasks.len(), before.tasks.len());
}

#[test]
fn done_ticks_exception_store_refusal_leaves_task_and_outcomes_unchanged() {
    use crate::application::project_board::ports::ProjectBoardStore;
    let (store, writer) = done_ticks_finished_board();
    let read = ok(&writer, json!({"op":"show","task":"a"}));
    let before = store.read().unwrap();
    let writes = store.writes();
    store.refuse_next_write("fixture refusal");
    let answer = call(&writer, done_ticks_request(&read["rev"]));
    assert_eq!(answer["ok"], false, "{answer}");
    assert_eq!(store.read().unwrap().tasks, before.tasks);
    assert_eq!(store.writes(), writes);
}

#[test]
fn done_ticks_exception_unrepresentable_reason_is_actionable_without_writes() {
    use crate::application::project_board::ports::ProjectBoardStore;
    let (store, writer) = done_ticks_finished_board();
    let read = ok(&writer, json!({"op":"show","task":"a"}));
    let before = store.read().unwrap();
    let writes = store.writes();
    let mut request = done_ticks_request(&read["rev"]);
    request["reason"] = json!("Owner `deferred");
    let answer = call(&writer, request);
    assert_eq!(answer["ok"], false, "{answer}");
    assert_eq!(store.read().unwrap().tasks, before.tasks);
    assert_eq!(
        store.writes(),
        writes,
        "unrepresentable reasons never write"
    );
    let diagnostic = answer["error"]["message"].as_str().unwrap();
    assert!(diagnostic.contains("reason"), "{answer}");
    assert!(diagnostic.contains("backtick"), "{answer}");
    assert!(diagnostic.contains("matching"), "{answer}");
}

#[test]
fn done_ticks_restore_already_met_is_a_no_write_no_revision_noop() {
    use crate::application::project_board::ports::ProjectBoardStore;
    let (store, writer) = done_ticks_finished_board();
    let read = ok(&writer, json!({"op":"show","task":"a"}));
    let writes = store.writes();
    let before = store.read().unwrap();
    let restored = ok(
        &writer,
        json!({"op":"set_outcome","task":"a","outcome_index":0,"met":true,"expected_rev":read["rev"]}),
    );
    assert_eq!(
        store.writes(),
        writes,
        "already met must not invoke persistence"
    );
    assert_eq!(store.read().unwrap().head, before.head, "no new revision");
    let reread = ok(&writer, json!({"op":"show","task":"a"}));
    assert_eq!(reread["rev"], read["rev"]);
    assert_eq!(restored["task"], read["task"]);
}

#[test]
fn done_ticks_invalid_index_has_its_own_diagnostic_not_paging_offset() {
    let (_store, writer) = done_ticks_finished_board();
    let read = ok(&writer, json!({"op":"show","task":"a"}));
    for index in [json!(-1), json!(0.5), json!("0")] {
        let mut request = done_ticks_request(&read["rev"]);
        request["outcome_index"] = index;
        let answer = call(&writer, request);
        assert_eq!(answer["error"]["field"], "outcome_index", "{answer}");
        let message = answer["error"]["message"].as_str().unwrap();
        assert!(message.contains("outcome_index"), "{answer}");
        assert!(
            message.contains("index"),
            "outcome diagnostic must name an index: {answer}"
        );
        assert_eq!(message.contains("read on from"), false, "{answer}");
    }
}

#[test]
fn done_ticks_public_answers_keep_lifecycle_reason_private() {
    let (_store, writer) = done_ticks_finished_board();
    let shown = ok(&writer, json!({"op":"show","task":"a"}));
    assert!(shown["task"].get("reason").is_none(), "{shown}");
    let listed = ok(&writer, json!({"op":"list"}));
    assert!(listed["tasks"][0].get("reason").is_none(), "{listed}");
}

#[test]
fn done_ticks_exception_and_restore_preserve_distinct_outcome_text_and_order() {
    let (_store, writer) = done_ticks_finished_board();
    ok(
        &writer,
        json!({"op":"create","task":"ordered","title":"Ordered outcomes","status":"ready","goal":"Ship safely.","done_when":["Zulu first","Alpha second","Middle last"]}),
    );
    ok(
        &writer,
        json!({"op":"done_without_run","task":"ordered","reason":"Answered by hand"}),
    );
    let original = ok(&writer, json!({"op":"show","task":"ordered"}));
    let changed = ok(
        &writer,
        json!({"op":"set_outcome","task":"ordered","outcome_index":1,"met":false,"reason":"Deferred","expected_rev":original["rev"]}),
    );
    let fresh = ok(&writer, json!({"op":"show","task":"ordered"}));
    let restored = ok(
        &writer,
        json!({"op":"set_outcome","task":"ordered","outcome_index":1,"met":true,"expected_rev":fresh["rev"]}),
    );
    for answer in [&changed, &restored] {
        let texts: Vec<_> = answer["task"]["done_when"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["outcome"].clone())
            .collect();
        assert_eq!(
            texts,
            vec![
                json!("Zulu first"),
                json!("Alpha second"),
                json!("Middle last")
            ]
        );
    }
    assert_eq!(
        restored["task"]["description"],
        original["task"]["description"]
    );
}

#[test]
fn done_ticks_stale_diagnostic_names_target_stamp_after_unrelated_write() {
    let (_store, writer) = done_ticks_finished_board();
    let original = ok(&writer, json!({"op":"show","task":"a"}));
    let changed = ok(&writer, done_ticks_request(&original["rev"]));
    ok(
        &writer,
        json!({"op":"create","task":"unrelated","title":"Another task"}),
    );
    let refused = call(
        &writer,
        json!({"op":"set_outcome","task":"a","outcome_index":0,"met":true,"expected_rev":original["rev"]}),
    );
    assert_eq!(refused["error"]["code"], "stale", "{refused}");
    assert_eq!(refused["error"]["task"], "a", "{refused}");
    assert_eq!(
        refused["error"]["updated"], changed["task"]["updated"],
        "target's revision stamp, not another task's: {refused}"
    );
}
