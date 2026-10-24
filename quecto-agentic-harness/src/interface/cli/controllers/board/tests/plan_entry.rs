//! A plan's approach on entry, by front end (#2705 T3): the agent tool
//! caps it and points the detail to the handoff; the CLI (people's, and
//! the Pocket relay's) keeps master's limits for a plan entered without a
//! handoff, and caps it once a request gives one.
use super::*;
use crate::domain::project_board::entities::plan::{MOVE_TO_HANDOFF, PLAN_ENTRY_RULES};
use crate::domain::project_board::value_objects::telemetry::{Client, FrontEnd};
use crate::interface::tools::project_board::recording::BoardCall;
use crate::interface::tools::project_board::run_recorded;

/// An approach `chars` characters long.
fn approach(chars: usize) -> String {
    "a".repeat(chars)
}

fn propose(approach: &str, handoff: Option<&str>) -> Value {
    let mut request = json!({"op": "propose_plan", "task": "a", "approach": approach,
                             "items": [{"id": "store", "title": "The store"}]});
    if let Some(handoff) = handoff {
        request["handoff"] = json!(handoff);
    }
    request
}

/// The ops of a board in memory with one ready task `a`.
fn board() -> Box<dyn crate::application::project_board::ports::BoardOperations> {
    use crate::application::project_board::ports::ProjectBoardStore;
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let operations = assembled(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .unwrap();
    let created = run(
        &operations,
        r#"{"op": "create", "task": "a", "title": "A", "status": "ready"}"#,
    );
    assert_eq!(created["ok"], json!(true), "{created}");
    Box::new(operations)
}

/// `request` answered as the CLI answers it, on `operations`.
fn as_cli(
    operations: &dyn crate::application::project_board::ports::BoardOperations,
    request: &Value,
) -> Value {
    let mut call = BoardCall::arrived(None, FrontEnd::Cli, Client::Agent);
    run_recorded(operations, &request.to_string(), &mut call)
}

#[test]
fn a_tool_approach_over_its_cap_is_refused_pointing_to_the_handoff() {
    let max = PLAN_ENTRY_RULES.max_approach_chars;
    let long = approach(max + 1);
    assert_eq!(long.chars().count(), max + 1);
    let operations = board();
    let answer = run(&*operations, &propose(&long, None).to_string());
    assert_eq!(
        (
            &answer["error"]["rule"]["code"],
            &answer["error"]["rule"]["field"]
        ),
        (&json!("invalid_plan"), &json!("approach")),
        "{answer}"
    );
    let problem = answer["error"]["rule"]["problem"].as_str().unwrap();
    assert_eq!(
        problem,
        format!(
            "is {} characters; the most is {max}: {MOVE_TO_HANDOFF}",
            max + 1
        )
    );
    // At the cap it enters, its detail in the handoff.
    let answer = run(
        &*operations,
        &propose(&approach(max), Some(&"Detail. ".repeat(2_000))).to_string(),
    );
    assert_eq!(answer["ok"], json!(true), "{answer}");
}

#[test]
fn a_cli_plan_without_a_handoff_keeps_masters_limits() {
    let long = approach(5_000);
    let operations = board();
    let answer = as_cli(&*operations, &propose(&long, None));
    assert_eq!(answer["ok"], json!(true), "{answer}");
    // An empty handoff is none (review round 1): no cap, and no handoff.
    let answer = as_cli(&*operations, &propose(&long, Some("")));
    assert_eq!(answer["ok"], json!(true), "{answer}");
    let shown = as_cli(
        &*operations,
        &json!({"op": "show", "task": "a", "version": 2}),
    );
    assert_eq!(shown["plan"].get("handoff"), None, "{shown}");
    // Given a handoff, the CLI caps the approach as the tool does.
    let answer = as_cli(&*operations, &propose(&long, Some("Detail.")));
    assert_eq!(
        answer["error"]["rule"]["field"],
        json!("approach"),
        "{answer}"
    );
}

/// An approach entered long before T3 (here by the CLI) is kept: an edit
/// through the tool that leaves it as it is enters nothing, and one that
/// gives it anew is capped.
#[test]
fn a_tool_edit_keeps_a_long_approach_it_does_not_give() {
    let long = approach(5_000);
    let operations = board();
    let proposed = as_cli(&*operations, &propose(&long, None));
    assert_eq!(proposed["ok"], json!(true), "{proposed}");
    let edit = json!({"op": "edit_plan", "task": "a", "version": 1,
                      "items": [{"id": "cli", "title": "The CLI"}]});
    let answer = run(&*operations, &edit.to_string());
    assert_eq!(answer["ok"], json!(true), "{answer}");
    let edit =
        json!({"op": "edit_plan", "task": "a", "version": 1, "approach": long.clone() + "!"});
    let answer = run(&*operations, &edit.to_string());
    assert_eq!(
        answer["error"]["rule"]["field"],
        json!("approach"),
        "{answer}"
    );
    let shown = run(&*operations, r#"{"op": "show", "task": "a", "version": 1}"#);
    assert_eq!(shown["plan"]["approach"], json!(long), "{shown}");
    assert_eq!(shown["plan"].get("handoff"), None, "{shown}");
}

/// A handoff given and then taken away (`""`): shown while it is held.
#[test]
fn a_handoff_is_shown_and_taken_away_by_an_empty_one() {
    let operations = board();
    let answer = run(
        &*operations,
        &propose("Outline.", Some("Detail\r\nhere.")).to_string(),
    );
    assert_eq!(answer["ok"], json!(true), "{answer}");
    let shown = run(&*operations, r#"{"op": "show", "task": "a", "version": 1}"#);
    assert_eq!(shown["plan"]["handoff"], json!("Detail\nhere."), "{shown}");
    let keys: Vec<&String> = shown["plan"].as_object().unwrap().keys().collect();
    assert_eq!(
        keys[3..6],
        ["approach", "handoff", "handoff_chars"],
        "{shown}"
    );
    assert_eq!(shown["plan"]["handoff_chars"], json!(12), "{shown}");
    let edit = json!({"op": "edit_plan", "task": "a", "version": 1, "handoff": ""});
    let answer = run(&*operations, &edit.to_string());
    assert_eq!(answer["ok"], json!(true), "{answer}");
    let shown = run(&*operations, r#"{"op": "show", "task": "a", "version": 1}"#);
    assert_eq!(shown["plan"].get("handoff"), None, "{shown}");
}
