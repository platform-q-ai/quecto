//! #2478 slice B: agents read task items. The tool describes, advertises
//! and answers in task-item words, as the owner approved them on
//! 2026-10-12; the board, its events, its refusals and the golden fixtures
//! keep the old names, so the answers are presented at the tool.
use serde_json::{Value, json};

use super::{answered, board, direct, execute, refused};
use crate::domain::swarm::owner::{OwnerState, owner_recovery};
use crate::infrastructure::tools::swarm::swarm_board_ops::{
    BOARD_OPS, ID_ALIAS, OP_ALIASES, advertised_arg, advertised_op, tool_schema,
};
use crate::infrastructure::tools::swarm::swarm_output::{
    task_item_answer, task_item_harness_answer, task_item_refusal,
};

const DESCRIPTION: &str = include_str!("swarm_assets/tool_description.txt");

/// The approved text of the description's lines 2 to 4, as far as it
/// changed (each line's rest is unchanged).
const APPROVED: [&str; 3] = [
    r#"Example: {"op":"claim","task_item_id":3} answers the task item with its claim token. Any member: task_item(task_item_id), task_items(offset,limit), file_owners(offset,limit), task_item_create(request,title,acceptance,dependencies), dependencies(task_item_id,dependencies), claim(task_item_id), release(task_item_id,token), block(task_item_id,token,reason), unblock(task_item_id,token,reason), submit(task_item_id,token,evidence), reserve(task_item_id,token,paths), release_files(task_item_id,token,reservation), send("#,
    "Any member may message any other member directly; a question about a task item you depend on belongs with that task item's owner. task_item, task_items and summary task_items carry owner_last_activity and owner_state (",
    "Coordinator only: amend(goal,constraints,criteria,reason), verify_task_item(task_item_id,token,revision), revalidate_task_item(task_item_id,revision,evidence), revoke(task_item_id,reason), recover(task_item_id,release_files) (after a confirmed death), ",
];

#[test]
fn the_description_reads_task_items_as_approved() {
    let lines: Vec<&str> = DESCRIPTION.lines().collect();
    assert_eq!(lines.len(), 4);
    for (line, approved) in lines[1..].iter().zip(APPROVED) {
        assert!(line.contains(approved), "{line}\nlacks\n{approved}");
    }
    // Every "task" left is a task item.
    for (at, _) in DESCRIPTION.match_indices("task") {
        let rest = &DESCRIPTION[at + "task".len()..];
        assert!(
            rest.starts_with(" item") || rest.starts_with("_item"),
            "{}",
            &DESCRIPTION[at..(at + 40).min(DESCRIPTION.len())]
        );
    }
}

/// The schema advertises the task-item names only, each in its old name's
/// place; the old names are still taken (#2478 A1) but not shown.
#[test]
fn the_schema_advertises_only_the_task_item_names() {
    let schema = tool_schema();
    let ops: Vec<&str> = schema["properties"]["op"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op.as_str().unwrap())
        .collect();
    assert_eq!(
        ops,
        [
            "create",
            "summary",
            "reconcile",
            "cancel_run",
            "pause",
            "resume",
            "events",
            "usage",
            "task_item",
            "task_items",
            "file_owners",
            "task_item_create",
            "dependencies",
            "claim",
            "release",
            "block",
            "unblock",
            "submit",
            "reserve",
            "release_files",
            "send",
            "withdraw",
            "inbox",
            "ack",
            "evidence",
            "amend",
            "verify_task_item",
            "revalidate_task_item",
            "recover",
            "revoke",
            "complete",
            "stop",
            "usage_report",
        ]
    );
    let properties = schema["properties"].as_object().unwrap();
    assert_eq!(properties["task_item_id"], json!({"type": "integer"}));
    assert!(!properties.contains_key("task_id"), "{properties:?}");
    let checked_in: Value =
        serde_json::from_str(include_str!("swarm_assets/tool_schema.json")).unwrap();
    assert_eq!(checked_in, schema);
}

#[test]
fn an_old_name_is_advertised_by_its_task_item_name() {
    for (alias, old) in OP_ALIASES {
        assert_eq!(advertised_op(old), *alias);
    }
    assert_eq!(advertised_op("claim"), "claim");
    assert_eq!(advertised_arg(ID_ALIAS.1), ID_ALIAS.0);
    assert_eq!(advertised_arg("token"), "token");
    for spec in BOARD_OPS {
        assert!(
            !advertised_op(spec.name).ends_with("_task"),
            "{}",
            spec.name
        );
    }
}

/// A task created, claimed and holding one file reservation, on the
/// coordinator's board; the claim's token and the reservation's.
async fn holding_a_file(
    context: &crate::infrastructure::tools::swarm::swarm_bridge::SwarmContext,
) -> (Value, Value) {
    answered(
        context,
        json!({"op": "task_item_create", "request": "r1", "title": "a", "acceptance": ["pass"]}),
    )
    .await;
    let claim = answered(context, json!({"op": "claim", "task_item_id": 1})).await;
    let token = claim["token"].clone();
    let reserved = answered(
        context,
        json!({"op": "reserve", "task_item_id": 1, "token": token, "paths": ["src/a.rs"]}),
    )
    .await;
    (token, reserved["token"].clone())
}

/// The harness op `op`'s answer, parsed.
async fn harness(
    context: &crate::infrastructure::tools::swarm::swarm_bridge::SwarmContext,
    request: Value,
) -> Value {
    let result = execute(context, &request.to_string()).await;
    assert!(!result.is_error, "{request}: {}", result.content);
    serde_json::from_str(&result.content).unwrap()
}

/// The answers name task items: the summary's `task_item_count` and
/// `task_items`, a file row's `task_item`, `release_files`'
/// `task_item_id`, and the events' `task_item_created` with
/// `{"task_item": N}`; the old keys are gone from what the member reads.
#[tokio::test]
async fn the_answers_name_task_items() {
    let (_directory, context) = board();
    let (token, reservation) = holding_a_file(&context).await;
    let summary = harness(&context, json!({"op": "summary"})).await;
    assert_eq!(summary["task_item_count"], 1, "{summary}");
    assert_eq!(summary["task_items"][0]["id"], 1, "{summary}");
    assert_eq!(summary["files"][0]["task_item"], 1, "{summary}");
    for old in ["task_count", "tasks"] {
        assert!(summary.get(old).is_none(), "{old}: {summary}");
    }
    assert!(summary["files"][0].get("task").is_none(), "{summary}");
    let owners = answered(&context, json!({"op": "file_owners"})).await;
    assert_eq!(owners[0]["task_item"], 1, "{owners}");
    assert!(owners[0].get("task").is_none(), "{owners}");
    let released = answered(
        &context,
        json!({"op": "release_files", "task_item_id": 1, "token": token,
            "reservation": reservation}),
    )
    .await;
    let keys: Vec<&str> = released
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys[..3],
        ["task_item_id", "reservation", "released"],
        "{released}"
    );
    let events = harness(&context, json!({"op": "events", "after": 0, "limit": 100})).await;
    let events = events["events"].as_array().unwrap();
    let created: Vec<&Value> = events
        .iter()
        .filter(|event| event["action"] == "task_item_created")
        .collect();
    assert_eq!(created.len(), 1, "{events:?}");
    assert_eq!(created[0]["detail"], r#"{"task_item":1}"#, "{events:?}");
    // A detail naming its task among other keys (sorted, as the board
    // writes them) names its task item in the same place.
    let reserved: Vec<&Value> = events
        .iter()
        .filter(|event| event["action"] == "files_reserved")
        .collect();
    assert_eq!(
        reserved[0]["detail"], r#"{"paths":["src/a.rs"],"task_item":1}"#,
        "{events:?}"
    );
    for event in events {
        assert_ne!(event["action"], "task_created", "{event}");
        let detail = event["detail"].as_str().unwrap_or_default();
        assert!(!detail.contains(r#""task":"#), "{event}");
    }
    // The board keeps its own names.
    let stored = direct(&context, "events", json!([0, 100])).unwrap();
    assert!(
        stored["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["action"] == "task_created"),
        "{stored}"
    );
}

/// A row's recovery hint is the coordinator's call, by its advertised
/// argument, for every owner state the board names one for.
#[test]
fn a_recovery_hint_names_the_call() {
    let expected = [
        (
            OwnerState::Dead,
            "recover(task_item_id) or revoke(task_item_id, reason)",
        ),
        (
            OwnerState::Lost,
            "resume the run (agent_cmd swarm_control resume), then revoke(task_item_id, reason)",
        ),
        (OwnerState::Active, "revoke(task_item_id, reason)"),
        (OwnerState::Idle, "revoke(task_item_id, reason)"),
        (OwnerState::Reserved, "revoke(task_item_id, reason)"),
        (OwnerState::Unknown, "revoke(task_item_id, reason)"),
    ];
    for (state, hint) in expected {
        let row = json!({"id": 1, "recovery": owner_recovery(state)});
        let presented = task_item_answer("task", row);
        assert_eq!(presented["recovery"], hint, "{state:?}");
        let page = task_item_answer(
            "tasks",
            json!([{"id": 1, "recovery": owner_recovery(state)}]),
        );
        assert_eq!(page[0]["recovery"], hint, "{state:?}");
        let summary = task_item_harness_answer(
            "summary",
            json!({"task_count": 1, "tasks": [{"id": 1, "recovery": owner_recovery(state)}],
                "files": []}),
        );
        assert_eq!(summary["task_items"][0]["recovery"], hint, "{state:?}");
    }
}

/// The coordinator's revoke notice reads `claim on task item N revoked`
/// in the inbox; any other body is as sent.
#[test]
fn the_revoke_notice_reads_task_item() {
    let inbox = json!([
        {"id": 1, "body": "claim on task 3 revoked by the coordinator: member silent"},
        {"id": 2, "body": "claim on task 3 is mine"},
    ]);
    let presented = task_item_answer("inbox", inbox);
    assert_eq!(
        presented[0]["body"],
        "claim on task item 3 revoked by the coordinator: member silent"
    );
    assert_eq!(presented[1]["body"], "claim on task 3 is mine");
}

/// The board's refusals the member reads name task items (#2478 D4): the
/// board keeps Python's text, which the golden fixtures pin, and the tool
/// presents it.
#[test]
fn the_refusals_name_task_items() {
    for (board, member) in [
        ("unknown task", "unknown task item"),
        (
            "task is not ready to claim",
            "task item is not ready to claim",
        ),
        (
            "task board full (500); settle existing work",
            "task item board full (500); settle existing work",
        ),
        (
            "task acceptance criteria required: use a nonempty list[str], e.g. ['tests pass']",
            "task item acceptance criteria required: use a nonempty list[str], e.g. ['tests pass']",
        ),
        (
            "task page requires nonnegative offset and limit 1 through 100",
            "task item page requires nonnegative offset and limit 1 through 100",
        ),
        (
            "task evidence refers to stale revision",
            "task item evidence refers to stale revision",
        ),
        (
            "only completed tasks may be revalidated",
            "only completed task items may be revalidated",
        ),
        (
            "claim: missing required argument task_id",
            "claim: missing required argument task_item_id",
        ),
        // The binding's refusals name the op as the schema advertises it.
        (
            "task: missing required argument task_id",
            "task_item: missing required argument task_item_id",
        ),
        (
            "task_create: missing required argument title",
            "task_item_create: missing required argument title",
        ),
        (
            "tasks: unexpected argument bogus",
            "task_items: unexpected argument bogus",
        ),
        (
            "verify_task: takes 3 arguments, 4 given",
            "verify_task_item: takes 3 arguments, 4 given",
        ),
        (
            "revalidate_task: arguments must be a JSON array or object",
            "revalidate_task_item: arguments must be a JSON array or object",
        ),
        (
            "task: task_id and task_item_id name the same argument but hold different values; send only one",
            "task_item: task_id and task_item_id name the same argument but hold different values; send only one",
        ),
        (
            "claim: unexpected argument owner",
            "claim: unexpected argument owner",
        ),
        // `recover`'s hints are the coordinator's calls too.
        (
            "recovery requires confirmed worker death; revoke(id, reason) reassigns a live owner",
            "recovery requires confirmed worker death; revoke(task_item_id, reason) reassigns a live owner",
        ),
        (
            "reservations retained after an abrupt exit; recover(id, release_files=True) frees them, or revoke(id, reason)",
            "reservations retained after an abrupt exit; recover(task_item_id, release_files=True) frees them, or revoke(task_item_id, reason)",
        ),
        // Anything else is the board's own.
        ("stale or unowned claim", "stale or unowned claim"),
        ("unknown taskmaster", "unknown taskmaster"),
        ("task: is done", "task: is done"),
        (
            "a task: missing required argument x",
            "a task: missing required argument x",
        ),
        // A member's own key that reads like a refusal is its own: only
        // the op is renamed.
        (
            "tasks: unexpected argument x: missing required argument task_id",
            "task_items: unexpected argument x: missing required argument task_id",
        ),
        (
            "claim: unexpected argument x: missing required argument task_id",
            "claim: unexpected argument x: missing required argument task_id",
        ),
        (
            "a b: missing required argument task_id",
            "a b: missing required argument task_id",
        ),
    ] {
        assert_eq!(task_item_refusal(board), member, "{board}");
    }
}

#[tokio::test]
async fn the_member_reads_the_refusals_in_task_items() {
    let (_directory, context) = board();
    for (request, expected) in [
        (
            json!({"op": "task_item", "task_item_id": 99}),
            r#"tool error: swarm: "unknown task item""#,
        ),
        (
            json!({"op": "task_items", "offset": 0, "limit": 0}),
            r#"tool error: swarm: "task item page requires nonnegative offset and limit 1 through 100""#,
        ),
        (
            json!({"op": "claim"}),
            r#"tool error: swarm: "claim: missing required argument task_item_id""#,
        ),
        (
            json!({"op": "task_item"}),
            r#"tool error: swarm: "task_item: missing required argument task_item_id""#,
        ),
        (
            json!({"op": "task"}),
            r#"tool error: swarm: "task_item: missing required argument task_item_id""#,
        ),
        (
            json!({"op": "task_item_create", "request": "r"}),
            r#"tool error: swarm: "task_item_create: missing required argument title""#,
        ),
        (
            json!({"op": "verify_task_item", "task_item_id": 1, "token": "t"}),
            r#"tool error: swarm: "verify_task_item: missing required argument revision""#,
        ),
        (
            json!({"op": "revalidate_task_item", "task_item_id": 1, "revision": "r"}),
            r#"tool error: swarm: "revalidate_task_item: missing required argument evidence""#,
        ),
        (
            json!({"op": "task_items", "bogus": 1}),
            r#"tool error: swarm: "task_items: unexpected argument bogus""#,
        ),
        (
            json!({"op": "task", "task_id": 1, "task_item_id": 2}),
            r#"tool error: swarm: "task_item: task_id and task_item_id name the same argument but hold different values; send only one""#,
        ),
        (
            json!({"op": "task_items", "x: missing required argument task_id": 1}),
            r#"tool error: swarm: "task_items: unexpected argument x: missing required argument task_id""#,
        ),
        (
            json!({"op": "claim", "task_item_id": 1, "x: missing required argument task_id": 1}),
            r#"tool error: swarm: "claim: unexpected argument x: missing required argument task_id""#,
        ),
    ] {
        assert_eq!(
            refused(&context, &request.to_string()).await,
            expected,
            "{request}"
        );
    }
}

/// The running gate names the op by its advertised name.
#[tokio::test]
async fn the_running_gate_names_the_advertised_op() {
    let (_directory, context) = board();
    direct(&context, "task_create", json!(["r1", "a", ["pass"]])).unwrap();
    crate::infrastructure::tools::registry::call_work::off_the_runtime(|| context.pause("hold"))
        .unwrap();
    for request in [
        json!({"op": "task", "task_id": 1}),
        json!({"op": "task_item", "task_item_id": 1}),
    ] {
        let refusal = refused(&context, &request.to_string()).await;
        assert!(
            refusal.contains("so op=task_item is unavailable"),
            "{refusal}"
        );
    }
}

/// A summary-shaped answer is presented only when it holds both keys; one
/// without `tasks` (or with `tasks` that is no list) is answered as is.
#[test]
fn a_partial_summary_is_answered_as_is() {
    for partial in [
        json!({"task_count": 0}),
        json!({"task_count": 0, "tasks": "none"}),
        json!({"tasks": []}),
    ] {
        assert_eq!(
            task_item_harness_answer("summary", partial.clone()),
            partial
        );
    }
}

/// The revoke notice is translated only when its task id is one the board
/// writes (a number, or plain text without spaces or punctuation).
#[test]
fn only_a_well_formed_revoke_notice_is_translated() {
    for body in [
        "claim on task  revoked by the coordinator: r",
        "claim on task 3:x revoked by the coordinator: r",
        "claim on task \"3\" revoked by the coordinator: r",
    ] {
        let inbox = task_item_answer("inbox", json!([{"body": body}]));
        assert_eq!(inbox[0]["body"], body);
    }
    for (id, presented) in [
        (
            "10000000000000000",
            "claim on task item 10000000000000000 revoked by the coordinator: r",
        ),
        (
            "1.5",
            "claim on task item 1.5 revoked by the coordinator: r",
        ),
        ("-2", "claim on task item -2 revoked by the coordinator: r"),
    ] {
        let body = format!("claim on task {id} revoked by the coordinator: r");
        let inbox = task_item_answer("inbox", json!([{"body": body}]));
        assert_eq!(inbox[0]["body"], presented);
    }
}

/// An event detail's top-level `task` key names its task item; a `task`
/// key nested deeper (in evidence, say) or text in a string is the board's
/// own, and the rest of the text is unchanged.
#[test]
fn only_the_top_level_task_key_of_a_detail_is_renamed() {
    for (detail, presented) in [
        (
            r#"{"evidence":[{"artifact":"a","revision":"r","task":9}],"revision":"r","task":1}"#,
            r#"{"evidence":[{"artifact":"a","revision":"r","task":9}],"revision":"r","task_item":1}"#,
        ),
        (
            r#"{"reason":"\"task\":1","task":2}"#,
            r#"{"reason":"\"task\":1","task_item":2}"#,
        ),
        (
            r#"{"evidence":[{"task":9}]}"#,
            r#"{"evidence":[{"task":9}]}"#,
        ),
        (r#"{"task" : 1}"#, r#"{"task_item" : 1}"#),
        (r#"["task",1]"#, r#"["task",1]"#),
        ("not json", "not json"),
        (r#"  {"task":1}"#, r#"  {"task_item":1}"#),
        // Text that is no whole object: a string running past the end, a
        // key after the object closed.
        (r#"{"reason":"open"#, r#"{"reason":"open"#),
        (r#"{"task\"#, r#"{"task\"#),
        (r#"{"a":1}{"task":2}"#, r#"{"a":1}{"task":2}"#),
    ] {
        let page = task_item_harness_answer(
            "events",
            json!({"events": [{"action": "revalidated", "detail": detail}]}),
        );
        assert_eq!(page["events"][0]["detail"], presented, "{detail}");
    }
}

/// Every op the presentation names is a board op, so a rename of the table
/// cannot leave it presenting nothing.
#[test]
fn every_presented_op_is_a_board_op() {
    use crate::infrastructure::tools::swarm::swarm_output::PRESENTED_OPS;
    assert!(!PRESENTED_OPS.is_empty());
    for (op, _) in PRESENTED_OPS {
        assert!(BOARD_OPS.iter().any(|spec| spec.name == *op), "{op}");
    }
}

/// A detail nested deeper than `serde_json` reads (its limit is 128) is
/// still presented: the scan reads it, not a parser.
#[test]
fn a_deep_detail_names_its_task_item() {
    for depth in [125, 200] {
        let lists = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        let detail = format!(r#"{{"evidence":{lists},"revision":"r","task":1}}"#);
        let page = task_item_harness_answer(
            "events",
            json!({"events": [{"action": "revalidated", "detail": detail}]}),
        );
        assert_eq!(
            page["events"][0]["detail"],
            format!(r#"{{"evidence":{lists},"revision":"r","task_item":1}}"#),
            "{depth}"
        );
    }
}

/// Every board op is presented (or deliberately not) as listed here, in
/// the table's order, so a board op added without a decision fails.
#[test]
fn every_board_op_has_a_presentation_decided() {
    use crate::infrastructure::tools::swarm::swarm_output::Presented::{
        FileRows, Messages, ReleasedFiles, TaskRow, TaskRows,
    };
    use crate::infrastructure::tools::swarm::swarm_output::presented;
    let decided = [
        ("task", Some(TaskRow)),
        ("tasks", Some(TaskRows)),
        ("file_owners", Some(FileRows)),
        ("task_create", Some(TaskRow)),
        ("dependencies", Some(TaskRow)),
        ("claim", Some(TaskRow)),
        ("release", Some(TaskRow)),
        ("block", Some(TaskRow)),
        ("unblock", Some(TaskRow)),
        ("submit", Some(TaskRow)),
        ("reserve", None),
        ("release_files", Some(ReleasedFiles)),
        ("send", None),
        ("withdraw", None),
        ("inbox", Some(Messages)),
        ("ack", None),
        ("evidence", None),
        ("amend", None),
        ("verify_task", Some(TaskRow)),
        ("revalidate_task", Some(TaskRow)),
        ("recover", Some(TaskRow)),
        ("revoke", Some(TaskRow)),
        ("complete", None),
        ("stop", None),
        ("usage_report", None),
    ];
    let found: Vec<_> = BOARD_OPS
        .iter()
        .map(|spec| (spec.name, presented(spec.name)))
        .collect();
    assert_eq!(found, decided);
}
