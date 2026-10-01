use serde_json::{Value, json};

use super::super::TELEMETRY_TARGET;
use super::super::tests::{board, captured, captured_on, running};
use crate::composition::swarm::SwarmBoardHandles;
use crate::domain::swarm::{BoardError, RefusalKind};
use crate::infrastructure::tools::swarm_board_dispatch::call;

/// The parent's claim of task 1: its token.
fn claimed(handles: &SwarmBoardHandles) -> Value {
    running(handles);
    call(handles, "parent", "task_create", json!(["r", "t", ["ok"]])).unwrap();
    call(handles, "parent", "claim", json!([1])).unwrap()["token"].clone()
}

/// `reserve` answers `{token, paths}` in that order, `file_owners` the rows
/// as `dict(row)`, `release_files` `{task_id, reservation, released}`
/// (#2394), and
/// `revoke` the task's dict.
#[test]
fn reservation_methods_render_pythons_shape() {
    let (_dir, handles) = board(1_000.0);
    let token = claimed(&handles);
    let reserved = call(&handles, "parent", "reserve", json!([1, token, ["b", "a"]])).unwrap();
    let ownership = reserved["token"].as_str().unwrap().to_owned();
    assert_eq!(
        serde_json::to_string(&reserved).unwrap(),
        format!(r#"{{"token":"{ownership}","paths":["a","b"]}}"#)
    );
    let page = call(&handles, "parent", "file_owners", json!({"limit": 1})).unwrap();
    assert_eq!(
        serde_json::to_string(&page).unwrap(),
        format!(
            r#"[{{"path":"a","task":1,"owner":"parent","claim":"{}","token":"{ownership}"}}]"#,
            token.as_str().unwrap()
        )
    );
    let released = call(
        &handles,
        "parent",
        "release_files",
        json!([1, token, ownership]),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&released).unwrap(),
        format!(r#"{{"task_id":1,"reservation":"{ownership}","released":2}}"#)
    );
    assert_eq!(
        call(&handles, "parent", "file_owners", json!([])).unwrap(),
        json!([])
    );
    let revoked = call(&handles, "parent", "revoke", json!([1, "reassign"])).unwrap();
    assert_eq!(
        (&revoked["id"], &revoked["status"], &revoked["owner"]),
        (&json!(1), &json!("ready"), &Value::Null)
    );
}

/// Each method binds Python's signature; `recover` defaults
/// `release_files` to `False`, and only JSON `true` releases.
#[test]
fn reservation_methods_bind_pythons_signatures() {
    let (_dir, handles) = board(1_000.0);
    claimed(&handles);
    for (method, args, message) in [
        (
            "reserve",
            json!([1, "t"]),
            "reserve: missing required argument paths",
        ),
        (
            "release_files",
            json!({"task_id": 1, "token": "t"}),
            "release_files: missing required argument reservation",
        ),
        (
            "file_owners",
            json!([0, 50, 1]),
            "file_owners: takes 2 arguments, 3 given",
        ),
        (
            "recover",
            json!({"release_files": true}),
            "recover: missing required argument task_id",
        ),
        (
            "revoke",
            json!([1]),
            "revoke: missing required argument reason",
        ),
    ] {
        assert_eq!(
            call(&handles, "parent", method, args).unwrap_err(),
            BoardError::new(RefusalKind::Calling, message)
        );
    }
    assert_eq!(
        call(&handles, "parent", "recover", json!([1])).unwrap_err(),
        BoardError::new(
            RefusalKind::WrongState,
            "recovery requires confirmed worker death; revoke(id, reason) reassigns a live owner"
        )
    );
}

/// Each call leaves one record with its decision (`file_owners` at DEBUG,
/// the rest at INFO) and no argument text: a secret-shaped path or reason
/// never reaches the log.
#[test]
fn reservation_calls_record_their_decisions_without_argument_text() {
    let secret = "sk-ant-api03-EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE";
    let log = captured(9, |handles| {
        let token = claimed(handles);
        let reserved = call(
            handles,
            "parent",
            "reserve",
            json!([1, token, [format!("src/{secret}.rs")]]),
        )
        .unwrap();
        call(handles, "parent", "file_owners", json!([])).unwrap();
        call(
            handles,
            "parent",
            "release_files",
            json!([1, token, reserved["token"]]),
        )
        .unwrap();
        call(handles, "parent", "recover", json!([1, true])).unwrap_err();
        call(
            handles,
            "parent",
            "revoke",
            json!([1, format!("see {secret}")]),
        )
        .unwrap();
        call(
            handles,
            "parent",
            "revoke",
            json!([1, format!("see {secret}")]),
        )
        .unwrap();
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET))
        .skip(3)
        .collect();
    let expected = [
        (" INFO ", "reserve", "ok", "reserved"),
        (" DEBUG ", "file_owners", "ok", "read"),
        (" INFO ", "release_files", "ok", "released"),
        (" INFO ", "recover", "refused", "none"),
        (" INFO ", "revoke", "ok", "revoked_notified"),
        (" INFO ", "revoke", "ok", "unchanged"),
    ];
    assert_eq!(records.len(), expected.len(), "{log}");
    for (record, (level, op, outcome, decision)) in records.iter().zip(expected) {
        for field in [
            level.to_owned(),
            format!("op=\"{op}\""),
            format!("outcome=\"{outcome}\""),
            format!("decision=\"{decision}\""),
            "member=\"parent\"".to_owned(),
            "duration_us=".to_owned(),
        ] {
            assert!(record.contains(&field), "{field} missing from {record}");
        }
    }
    assert!(!log.contains(secret), "{log}");
    assert!(!log.contains("sk-ant"), "{log}");
    assert!(!log.contains("see "), "argument text never logged: {log}");
}

/// `recover`'s decision names whether it released reservations with the
/// task (#2321 mutation report): none retained is `recovered`, one or
/// more `recovered_releasing_files`.
#[test]
fn recover_records_whether_it_released_reservations() {
    let log = captured_on(10, |handles, database| {
        running(handles);
        call(handles, "parent", "_admit", json!(["worker", "r1"])).unwrap();
        call(
            handles,
            "parent",
            "_activate",
            json!(["worker", "r1", 7, "s", "/w.sock"]),
        )
        .unwrap();
        for request in ["a", "b"] {
            call(
                handles,
                "parent",
                "task_create",
                json!([request, "t", ["ok"]]),
            )
            .unwrap();
        }
        let token = call(handles, "worker", "claim", json!([1])).unwrap()["token"].clone();
        call(handles, "worker", "claim", json!([2])).unwrap();
        call(handles, "worker", "reserve", json!([1, token, ["a.rs"]])).unwrap();
        rusqlite::Connection::open(database)
            .unwrap()
            .execute("UPDATE members SET status='dead' WHERE id='worker'", [])
            .unwrap();
        call(handles, "parent", "recover", json!([2])).unwrap();
        call(handles, "parent", "recover", json!([1, true])).unwrap();
    });
    let decisions: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET) && line.contains("op=\"recover\""))
        .map(|line| {
            let (_, decision) = line.split_once("decision=\"").expect("a decision");
            decision.split('"').next().expect("a closing quote")
        })
        .collect();
    assert_eq!(
        decisions,
        ["recovered", "recovered_releasing_files"],
        "{log}"
    );
}
