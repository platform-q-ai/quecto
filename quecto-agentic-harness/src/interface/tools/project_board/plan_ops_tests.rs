//! #2481 M4b review round 1: a draft is read whole with `show(task,
//! version)`, so another agent builds on it without wiping it.
use std::sync::Arc;

use serde_json::{Value, json};

use super::run;
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::ports::{ArtifactKey, ProjectBoardStore};
use crate::application::project_board::use_cases::QueryBoard;
use crate::composition::bootstrap::project_board_operations::LocalBoardOperations;
use crate::composition::bootstrap::project_board_operations::fixtures::{QueryListings, assembled};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;

/// The board's operations as `name` writes them.
fn as_writer(store: &MemoryBoardStore, name: &str) -> LocalBoardOperations {
    let client = store.client_as(Identity {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase()),
    });
    assembled(
        Arc::new(client.clone()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(client)))),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .unwrap()
}

fn call(operations: &LocalBoardOperations, request: Value) -> Value {
    run(operations, &request.to_string())
}

fn ok(operations: &LocalBoardOperations, request: Value) -> Value {
    let answer = call(operations, request);
    assert_eq!(answer["ok"], json!(true), "{answer}");
    answer
}

fn item(id: &str) -> Value {
    json!({"id": id, "title": format!("Item {id}"), "criteria": [format!("{id} proved")], "depends_on": []})
}

/// A board with task `a` and two drafts of its plan by Ada, the second
/// with two task items.
fn drafted() -> (MemoryBoardStore, LocalBoardOperations, LocalBoardOperations) {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let (ada, bob) = (as_writer(&store, "Ada"), as_writer(&store, "Bob"));
    ok(&ada, json!({"op": "create", "task": "a", "title": "A"}));
    ok(
        &ada,
        json!({"op": "propose_plan", "task": "a", "approach": "First.", "items": []}),
    );
    ok(
        &ada,
        json!({"op": "propose_plan", "task": "a", "approach": "Ada's approach.",
               "items": [item("schema"), item("api")]}),
    );
    (store, ada, bob)
}

#[test]
fn bob_reads_adas_draft_and_adds_one_item_without_wiping_hers() {
    let (_store, ada, bob) = drafted();
    let draft = ok(&bob, json!({"op": "show", "task": "a", "version": 2}));
    let plan = &draft["plan"];
    assert_eq!(
        (&plan["version"], &plan["state"], &plan["approach"]),
        (&json!(2), &json!("draft"), &json!("Ada's approach.")),
        "{draft}"
    );
    assert_eq!(plan["proposed_by"]["name"], json!("Ada"));
    let mut items = plan["items"].as_array().unwrap().clone();
    assert_eq!(items, vec![item("schema"), item("api")]);
    items.push(item("cli"));
    let edited = ok(
        &bob,
        json!({"op": "edit_plan", "task": "a", "version": 2, "items": items,
               "expected_updated": plan["updated"]}),
    );
    assert_eq!(edited["plan"]["edited_by"]["name"], json!("Bob"));
    let after = ok(&ada, json!({"op": "show", "task": "a", "version": 2}));
    assert_eq!(
        after["plan"]["items"],
        json!([item("schema"), item("api"), item("cli")])
    );
    assert_eq!(after["plan"]["approach"], json!("Ada's approach."));
    // The accepted plan is still none: a version named is shown in its
    // place only when asked for.
    assert_eq!(
        ok(&ada, json!({"op": "show", "task": "a"}))["plan"],
        Value::Null
    );
}

/// A version is paged as the accepted plan is, and its page names it.
#[test]
fn a_named_version_is_paged() {
    let (_store, _ada, bob) = drafted();
    let page = ok(
        &bob,
        json!({"op": "show", "task": "a", "version": 2, "items_from": 1, "approach_from": 6}),
    );
    let whole = ok(&bob, json!({"op": "show", "task": "a", "version": 2}));
    assert_eq!(
        page["plan"],
        json!({"version": 2, "updated": whole["plan"]["updated"], "approach_from": 6,
               "approach": "approach.", "items_from": 1, "items": [item("api")]})
    );
}

/// Review round 2: a page names the version's `updated`, so a reader
/// paging through a draft sees when another edit landed between its
/// pages, and reads again rather than mixing two edits.
#[test]
fn a_page_after_an_edit_names_the_new_updated() {
    let (_store, ada, bob) = drafted();
    let first = ok(
        &bob,
        json!({"op": "show", "task": "a", "version": 2, "approach_from": 0}),
    );
    ok(
        &ada,
        json!({"op": "edit_plan", "task": "a", "version": 2, "items": [item("other")]}),
    );
    let next = ok(
        &bob,
        json!({"op": "show", "task": "a", "version": 2, "items_from": 0}),
    );
    let (was, now) = (&first["plan"]["updated"], &next["plan"]["updated"]);
    assert!(was.is_string() && now.is_string(), "{first} {next}");
    assert_ne!(was, now, "the edit between the pages shows");
    assert_eq!(next["plan"]["items"], json!([item("other")]));
}

/// Review round 2: a plan's text past the most a plan holds is the
/// domain's one refusal, `invalid_plan` naming `text`, with the count.
#[test]
fn plan_text_past_the_most_reaches_the_agent_as_invalid_plan_text() {
    let (store, ada, _bob) = drafted();
    let writes = store.writes();
    let criteria: Vec<String> = (b'a'..b'q')
        .map(|letter| char::from(letter).to_string().repeat(500))
        .collect();
    let items: Vec<Value> = (0..8)
        .map(|index| json!({"id": format!("i{index}"), "title": "t".repeat(500), "criteria": criteria}))
        .collect();
    let refused = call(
        &ada,
        json!({"op": "propose_plan", "task": "a", "approach": "\u{1f600}".repeat(65_536),
               "items": items}),
    );
    let rule = &refused["error"]["rule"];
    assert_eq!(
        (&rule["code"], &rule["field"], &rule["problem"]),
        (
            &json!("invalid_plan"),
            &json!("text"),
            &json!("the plan's text is 330144 bytes; the most is 327680")
        ),
        "{refused}"
    );
    assert_eq!(store.writes(), writes);
}

/// A version the task does not have, or whose file is set aside, is
/// refused, never shown as none.
#[test]
fn an_unknown_or_set_aside_version_is_refused() {
    let (store, _ada, bob) = drafted();
    let unknown = call(&bob, json!({"op": "show", "task": "a", "version": 9}));
    assert_eq!(
        (
            &unknown["error"]["rule"]["code"],
            &unknown["error"]["message"]
        ),
        (
            &json!("unknown_plan_version"),
            &json!("the task has no plan version 9")
        ),
        "{unknown}"
    );
    let key = ArtifactKey::Plan(Slug::parse("a").unwrap(), 2);
    store.quarantine_key(key, "plans/a.2.md", "unreadable");
    let set_aside = call(&bob, json!({"op": "show", "task": "a", "version": 2}));
    assert_eq!(
        (&set_aside["error"]["code"], &set_aside["error"]["path"]),
        (&json!("artifact_set_aside"), &json!("plans/a.2.md")),
        "{set_aside}"
    );
}

/// A plan the schema refuses is named a plan, not a task.
#[test]
fn a_plans_schema_refusal_says_plan() {
    let (store, ada, _bob) = drafted();
    let writes = store.writes();
    let refused = call(
        &ada,
        json!({"op": "edit_plan", "task": "a", "version": 2,
               "items": [{"id": "x", "title": "Two\nlines"}]}),
    );
    let error = &refused["error"];
    assert_eq!(
        (
            &error["code"],
            &error["rule"]["code"],
            &error["rule"]["field"]
        ),
        (
            &json!("rule"),
            &json!("invalid_plan"),
            &json!("items/0/title")
        ),
        "{refused}"
    );
    let message = error["message"].as_str().unwrap();
    assert!(
        message.starts_with("invalid plan: items/0/title: "),
        "{message}"
    );
    assert_eq!(store.writes(), writes);
}

/// Review round 3: an edit that gives only task items, past the most a
/// plan holds beside the large approach the draft keeps, is refused by
/// the domain as `invalid_plan` naming `text`, writes nothing, and is
/// recorded as the request as a whole, too large.
#[test]
fn an_items_only_edit_past_the_most_beside_a_kept_approach_is_refused() {
    use super::recording::BoardCall;
    use super::recording::tests::{Records, recording};
    use super::run_recorded;
    use crate::domain::audit::AuditEvent;
    use crate::domain::project_board::value_objects::telemetry::{Client, FrontEnd};
    let (store, ada, _bob) = drafted();
    ok(
        &ada,
        json!({"op": "propose_plan", "task": "a", "approach": "\u{1f600}".repeat(65_536),
               "items": []}),
    );
    let writes = store.writes();
    let criteria: Vec<String> = (b'a'..b'q')
        .map(|letter| char::from(letter).to_string().repeat(500))
        .collect();
    let items: Vec<Value> = (0..8)
        .map(|index| json!({"id": format!("i{index}"), "title": "t".repeat(500), "criteria": criteria}))
        .collect();
    let edit = json!({"op": "edit_plan", "task": "a", "version": 3, "items": items});
    let records = Arc::new(Records::default());
    let mut call = BoardCall::start(Some(recording(&records)), FrontEnd::Tool, Client::Agent);
    let refused = run_recorded(&ada, &edit.to_string(), &mut call);
    call.finish(&refused, false);
    let rule = &refused["error"]["rule"];
    assert_eq!(
        (&rule["code"], &rule["field"]),
        (&json!("invalid_plan"), &json!("text")),
        "{refused}"
    );
    assert_eq!(store.writes(), writes, "nothing written");
    let line =
        serde_json::to_value(AuditEvent::BoardOp(Box::new(records.taken().remove(0)))).unwrap();
    assert_eq!(
        [&line["code"], &line["rule"], &line["field"], &line["fault"]],
        [
            &json!("rule"),
            &json!("invalid_plan"),
            &json!("request"),
            &json!("too_large")
        ],
        "{line}"
    );
}

/// #2481 M4c: Bob reads Ada's draft and accepts it as read; the board
/// names it, a later acceptance supersedes it, and the call is recorded
/// with the plan's states and version, never its text.
#[test]
fn a_draft_read_is_accepted_superseded_and_recorded_by_state() {
    use super::recording::BoardCall;
    use super::recording::tests::{Records, recording};
    use super::run_recorded;
    use crate::domain::audit::AuditEvent;
    use crate::domain::project_board::value_objects::telemetry::{Client, FrontEnd};
    let (_store, ada, bob) = drafted();
    let draft = ok(&bob, json!({"op": "show", "task": "a", "version": 2}));
    let updated = draft["plan"]["updated"].clone();
    let records = Arc::new(Records::default());
    let mut recorded = BoardCall::start(Some(recording(&records)), FrontEnd::Tool, Client::Agent);
    let request = json!({"op": "accept_plan", "task": "a", "version": 2, "expected_updated": updated, "expected_accepted": null});
    let accepted = run_recorded(&bob, &request.to_string(), &mut recorded);
    recorded.finish(&accepted, false);
    assert_eq!(accepted["ok"], json!(true), "{accepted}");
    assert_eq!(
        (
            &accepted["task"]["accepted_plan"],
            &accepted["plan"]["state"],
            &accepted["superseded"]
        ),
        (&json!(2), &json!("accepted"), &Value::Null)
    );
    let line =
        serde_json::to_value(AuditEvent::BoardOp(Box::new(records.taken().remove(0)))).unwrap();
    assert_eq!(
        [
            &line["op"],
            &line["plan_version"],
            &line["plan_state_from"],
            &line["plan_state_to"]
        ],
        [
            &json!("accept_plan"),
            &json!(2),
            &json!("draft"),
            &json!("accepted")
        ],
        "{line}"
    );
    let first = ok(&ada, json!({"op": "show", "task": "a", "version": 1}));
    let again = ok(
        &ada,
        json!({"op": "accept_plan", "task": "a", "version": 1, "expected_accepted": 2,
               "expected_updated": first["plan"]["updated"]}),
    );
    assert_eq!(
        again["superseded"],
        json!(2),
        "a rollback to the older draft"
    );
    let shown = ok(&ada, json!({"op": "show", "task": "a"}));
    assert_eq!(shown["plan"]["version"], json!(1));
    let discard = call(
        &ada,
        json!({"op": "discard_plan", "task": "a", "version": 2}),
    );
    assert_eq!(
        discard["error"]["rule"]["code"],
        json!("plan_not_draft"),
        "a superseded plan is not discarded"
    );
}
