use serde_json::{Value, json};

use super::super::TELEMETRY_TARGET;
use super::super::tests::{board, captured, running};
use crate::domain::swarm::{BoardError, RefusalKind};
use crate::infrastructure::tools::swarm_board_dispatch::{SwarmBoardHandles, call};

/// A running run of three coordinated by `parent`, with `worker` live.
fn staffed(handles: &SwarmBoardHandles) {
    running(handles);
    call(handles, "parent", "_admit", json!(["worker", "r"])).unwrap();
    call(
        handles,
        "parent",
        "_activate",
        json!(["worker", "r", 7, "t", null]),
    )
    .unwrap();
}

fn keys(value: &Value) -> Vec<&str> {
    value
        .as_object()
        .expect("a dict")
        .keys()
        .map(String::as_str)
        .collect()
}

const COLUMNS: [&str; 9] = [
    "id",
    "title",
    "acceptance",
    "dependencies",
    "status",
    "owner",
    "token",
    "evidence",
    "blocker",
];

/// The task's dict keeps `SELECT *` column order; a replayed request
/// answers the stored text as `json.loads` reads it, keys sorted; `claim`
/// answers the claimed task with its token; `dependencies` and `release`
/// answer the task's dict as it now stands (#2394), in the same order.
#[test]
fn task_methods_render_pythons_shape() {
    let (_dir, handles) = board(1_000.0);
    staffed(&handles);
    let created = call(
        &handles,
        "worker",
        "task_create",
        json!(["r1", "first", ["tests pass"]]),
    )
    .unwrap();
    assert_eq!(keys(&created), COLUMNS);
    assert_eq!(
        created,
        json!({"id": 1, "title": "first", "acceptance": ["tests pass"], "dependencies": [],
               "status": "ready", "owner": null, "token": null, "evidence": [], "blocker": null})
    );
    let replayed = call(
        &handles,
        "worker",
        "task_create",
        json!({"acceptance": ["tests pass"], "title": "first", "request": "r1", "dependencies": []}),
    )
    .unwrap();
    let mut sorted = COLUMNS;
    sorted.sort_unstable();
    assert_eq!(keys(&replayed), sorted);
    assert_eq!(replayed, created);
    let second = call(
        &handles,
        "worker",
        "task_create",
        json!(["r2", "second", ["ok"], [1]]),
    )
    .unwrap();
    assert_eq!(second["status"], json!("blocked"));
    let changed = call(&handles, "worker", "dependencies", json!([2, []])).unwrap();
    assert_eq!(keys(&changed), COLUMNS);
    assert_eq!(
        (&changed["id"], &changed["status"], &changed["dependencies"]),
        (&json!(2), &json!("ready"), &json!([]))
    );
    let claimed = call(&handles, "worker", "claim", json!({"task_id": 1})).unwrap();
    assert_eq!(keys(&claimed), COLUMNS);
    assert_eq!(claimed["owner"], json!("worker"));
    let token = claimed["token"].clone();
    assert_eq!(token.as_str().map(str::len), Some(32));
    assert_eq!(
        call(&handles, "worker", "task_raw", json!([1])).unwrap(),
        claimed
    );
    let released = call(&handles, "worker", "release", json!([1, token])).unwrap();
    assert_eq!(keys(&released), COLUMNS);
    assert_eq!(
        call(&handles, "worker", "task_raw", json!([1])).unwrap(),
        released
    );
    assert_eq!(
        (&released["status"], &released["owner"], &released["token"]),
        (&json!("ready"), &Value::Null, &Value::Null)
    );
    assert_eq!(
        call(&handles, "worker", "task_create", json!(["r3", "t"])).unwrap_err(),
        BoardError::new(
            RefusalKind::Calling,
            "task_create: missing required argument acceptance"
        )
    );
    assert_eq!(
        call(&handles, "worker", "claim", json!([1, 2])).unwrap_err(),
        BoardError::new(RefusalKind::Calling, "claim: takes 1 arguments, 2 given")
    );
}

/// Each task call leaves one record with its decision, at INFO for a
/// mutation and DEBUG for the raw read, and no argument text: a
/// secret-shaped request id, title, acceptance or token never reaches the
/// log.
#[test]
fn task_calls_record_their_decisions_without_argument_text() {
    let secret = "sk-ant-api03-EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE";
    let log = captured(9, |handles| {
        staffed(handles);
        let args = json!([secret, format!("use {secret}"), [format!("key {secret}")]]);
        call(handles, "worker", "task_create", args.clone()).unwrap();
        call(handles, "worker", "task_create", args).unwrap();
        call(handles, "worker", "dependencies", json!([1, []])).unwrap();
        call(handles, "worker", "claim", json!([1])).unwrap();
        call(handles, "worker", "release", json!([1, secret])).unwrap_err();
        call(handles, "worker", "task_raw", json!([1])).unwrap();
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET))
        .skip(3)
        .collect();
    let expected = [
        (" INFO ", "task_create", "ok", "created"),
        (" INFO ", "task_create", "ok", "replayed"),
        (" INFO ", "dependencies", "ok", "updated"),
        (" INFO ", "claim", "ok", "claimed"),
        (" INFO ", "release", "refused", "none"),
        ("DEBUG ", "task_raw", "ok", "read"),
    ];
    assert_eq!(records.len(), expected.len(), "{log}");
    for (record, (level, op, outcome, decision)) in records.iter().zip(expected) {
        for field in [
            level.to_owned(),
            format!("op=\"{op}\""),
            format!("outcome=\"{outcome}\""),
            format!("decision=\"{decision}\""),
            "member=\"worker\"".to_owned(),
            "duration_us=".to_owned(),
        ] {
            assert!(record.contains(&field), "{field} missing from {record}");
        }
    }
    assert!(!log.contains(secret), "{log}");
    assert!(!log.contains("sk-ant"), "{log}");
    assert!(!log.contains("use "), "argument text never logged: {log}");
}
