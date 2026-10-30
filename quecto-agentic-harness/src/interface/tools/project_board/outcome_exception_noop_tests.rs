//! Repeated outcome exceptions preserve original attribution without writes.
use serde_json::json;

use super::plan_ops_rev_tests::{done_ticks_finished_board, done_ticks_request};
use super::plan_ops_tests::ok;

/// Repeating an exception is a semantic no-op even for a different trusted
/// writer. Use distinct outcome text so whole-task equality also guards order.
fn assert_done_ticks_exception_same_reason_is_noop(reason: &str) {
    use crate::application::project_board::ports::ProjectBoardStore;
    let (store, ada) = done_ticks_finished_board();
    ok(
        &ada,
        json!({"op":"create","task":"ordered","title":"Ordered outcomes","status":"ready",
            "goal":"Ship safely.","done_when":["Zulu first","Alpha second","Middle last"]}),
    );
    ok(
        &ada,
        json!({"op":"done_without_run","task":"ordered","reason":"Answered by hand"}),
    );
    let original = ok(&ada, json!({"op":"show","task":"ordered"}));
    ok(
        &ada,
        json!({"op":"set_outcome","task":"ordered","outcome_index":1,"met":false,
            "reason":"Owner deferred deployment","expected_rev":original["rev"]}),
    );
    let bob = super::plan_ops_tests::as_writer(&store, "Bob");
    let current = ok(&bob, json!({"op":"show","task":"ordered"}));
    assert_eq!(current["task"]["status"], "done");
    assert_eq!(current["task"]["done_when"][1]["done"], false);
    let exception = &current["task"]["done_when"][1]["exception"];
    assert_eq!(exception["reason"], "Owner deferred deployment");
    assert_eq!(
        exception["by"],
        json!({"name":"Ada","email":"ada@example.com"})
    );
    assert_eq!(exception["at"], current["task"]["updated"]);
    let writes = store.writes();
    let before = store.read().unwrap();

    let repeated = ok(
        &bob,
        json!({"op":"set_outcome","task":"ordered","outcome_index":1,"met":false,
            "reason":reason,"expected_rev":current["rev"]}),
    );

    assert_eq!(
        store.writes(),
        writes,
        "same normalized exception must not invoke persistence"
    );
    let after = store.read().unwrap();
    assert_eq!(after.head, before.head, "same exception keeps board head");
    assert_eq!(
        after.tasks, before.tasks,
        "stored task state stays unchanged"
    );
    let reread = ok(&bob, json!({"op":"show","task":"ordered"}));
    assert_eq!(
        reread["rev"], current["rev"],
        "task revision stays unchanged"
    );
    for answer in [&repeated, &reread] {
        assert_eq!(answer["task"]["updated"], current["task"]["updated"]);
        assert_eq!(
            answer["task"]["done_when"][1]["exception"], *exception,
            "original reason, attribution and time survive another writer's retry"
        );
        assert_eq!(
            answer["task"]["done_when"], current["task"]["done_when"],
            "outcome text, order and met states stay unchanged"
        );
        assert_eq!(
            answer["task"], current["task"],
            "all public task state stays unchanged"
        );
    }
}

#[test]
fn done_ticks_exception_same_exact_reason_is_a_no_write_no_revision_noop() {
    assert_done_ticks_exception_same_reason_is_noop("Owner deferred deployment");
}

#[test]
fn done_ticks_exception_same_normalized_reason_is_a_no_write_no_revision_noop() {
    assert_done_ticks_exception_same_reason_is_noop("  Owner\u{a0}deferred deployment  ");
}

#[test]
fn done_ticks_exception_different_reason_still_writes_and_records_new_attribution() {
    use crate::application::project_board::ports::ProjectBoardStore;
    let (store, ada) = done_ticks_finished_board();
    let original = ok(&ada, json!({"op":"show","task":"a"}));
    ok(&ada, done_ticks_request(&original["rev"]));
    let bob = super::plan_ops_tests::as_writer(&store, "Bob");
    let current = ok(&bob, json!({"op":"show","task":"a"}));
    let before = store.read().unwrap();
    let writes = store.writes();
    let mut request = done_ticks_request(&current["rev"]);
    request["reason"] = json!("Owner cancelled deployment");

    let changed = ok(&bob, request);

    assert_eq!(store.writes(), writes + 1, "different reason persists once");
    assert_ne!(store.read().unwrap().head, before.head);
    let reread = ok(&bob, json!({"op":"show","task":"a"}));
    assert_ne!(reread["rev"], current["rev"]);
    assert_eq!(changed["task"], reread["task"]);
    assert_eq!(changed["task"]["status"], "done");
    assert_eq!(changed["task"]["done_when"][0]["done"], false);
    let exception = &changed["task"]["done_when"][0]["exception"];
    assert_eq!(exception["reason"], "Owner cancelled deployment");
    assert_eq!(
        exception["by"],
        json!({"name":"Bob","email":"bob@example.com"})
    );
    assert_eq!(exception["at"], changed["task"]["updated"]);
    assert_eq!(
        changed["task"]["done_when"][1],
        current["task"]["done_when"][1]
    );
}
