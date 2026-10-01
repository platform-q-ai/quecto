//! #2394: every member-facing board op that changes a task answers the
//! task's row as it now stands (the shape `claim` answers: status, owner,
//! and the claim token while the claim is held), `evidence` answers the
//! evidence it recorded, and the ops that change no task answer a small
//! typed result: no member-facing op answers a bare `null`.
use serde_json::{Value, json};

use super::tests::{board, board_and_repository, running};
use super::{SwarmBoardHandles, call};

/// A running run of three coordinated by `parent`, with `worker` live:
/// `worker` holds the claim of task 1; task 2 is ready. The claim token.
fn claimed(handles: &SwarmBoardHandles) -> Value {
    running(handles);
    call(handles, "parent", "_admit", json!(["worker", "r"])).unwrap();
    call(
        handles,
        "parent",
        "_activate",
        json!(["worker", "r", 7, "t", null]),
    )
    .unwrap();
    for request in ["r1", "r2"] {
        call(
            handles,
            "worker",
            "task_create",
            json!([request, "work", ["tests pass"]]),
        )
        .unwrap();
    }
    call(handles, "worker", "claim", json!([1])).unwrap()["token"].clone()
}

/// The task's row as the board now holds it (`claim`'s shape, without
/// the owner's liveness that `task` adds).
fn row(handles: &SwarmBoardHandles, task: i64) -> Value {
    call(handles, "worker", "task_raw", json!([task])).unwrap()
}

/// `answer` is task `task`'s row as it now stands, with `status`, `owner`
/// and `token`: the fields a member confirms the effect with.
#[track_caller]
fn assert_row(
    handles: &SwarmBoardHandles,
    op: &str,
    answer: &Value,
    task: i64,
    (status, owner, token): (&str, Value, Value),
) {
    assert!(answer.is_object(), "{op} answers the task row: {answer}");
    assert_eq!(answer["id"], json!(task), "{op}: {answer}");
    assert_eq!(answer["status"], json!(status), "{op}: {answer}");
    assert_eq!(answer["owner"], owner, "{op}: {answer}");
    assert_eq!(answer["token"], token, "{op}: {answer}");
    assert_eq!(
        answer,
        &row(handles, task),
        "{op}: the row as it now stands"
    );
}

#[test]
fn block_unblock_submit_and_verify_task_answer_the_task_row() {
    let (_dir, handles) = board(1_000.0);
    let token = claimed(&handles);
    let worker = json!("worker");
    let evidence = json!([{"artifact": "report", "revision": "R1"}]);
    for (member, op, args, status) in [
        ("worker", "block", json!([1, token, "waiting"]), "blocked"),
        // The same blocker again changes nothing, and still answers the row.
        ("worker", "block", json!([1, token, "waiting"]), "blocked"),
        (
            "worker",
            "unblock",
            json!([1, token, "resolved"]),
            "claimed",
        ),
        ("worker", "submit", json!([1, token, evidence]), "submitted"),
        (
            "parent",
            "verify_task",
            json!([1, token, "R1"]),
            "completed",
        ),
        (
            "parent",
            "verify_task",
            json!([1, token, "R1"]),
            "completed",
        ),
    ] {
        let answer = call(&handles, member, op, args).unwrap();
        assert_row(
            &handles,
            op,
            &answer,
            1,
            (status, worker.clone(), token.clone()),
        );
    }
    let submitted = row(&handles, 1);
    assert_eq!(submitted["evidence"], evidence, "submit's evidence is kept");
}

#[test]
fn release_answers_the_ready_row_without_owner_or_token() {
    let (_dir, handles) = board(1_000.0);
    let token = claimed(&handles);
    let answer = call(&handles, "worker", "release", json!([1, token])).unwrap();
    assert_row(
        &handles,
        "release",
        &answer,
        1,
        ("ready", Value::Null, Value::Null),
    );
}

#[test]
fn dependencies_answers_the_row_with_its_new_dependencies() {
    let (_dir, handles) = board(1_000.0);
    claimed(&handles);
    let answer = call(&handles, "worker", "dependencies", json!([2, [1]])).unwrap();
    // Task 1 is not completed, so task 2 reads blocked by it.
    assert_row(
        &handles,
        "dependencies",
        &answer,
        2,
        ("blocked", Value::Null, Value::Null),
    );
    assert_eq!(answer["dependencies"], json!([1]), "{answer}");
    assert_eq!(answer["blocker"], json!("unmet dependencies"), "{answer}");
}

#[test]
fn revalidate_task_answers_the_row_with_the_new_evidence() {
    let (_dir, handles) = board(1_000.0);
    let token = claimed(&handles);
    let first = json!([{"artifact": "report", "revision": "R1"}]);
    call(&handles, "worker", "submit", json!([1, token, first])).unwrap();
    call(&handles, "parent", "verify_task", json!([1, token, "R1"])).unwrap();
    let again = json!([{"artifact": "rerun", "revision": "R2"}]);
    let answer = call(
        &handles,
        "parent",
        "revalidate_task",
        json!([1, "R2", again]),
    )
    .unwrap();
    assert_row(
        &handles,
        "revalidate_task",
        &answer,
        1,
        ("completed", json!("worker"), token),
    );
    assert_eq!(answer["evidence"], again, "{answer}");
}

#[test]
fn recover_answers_the_reopened_row() {
    let (dir, handles, _) = board_and_repository(1_000.0);
    claimed(&handles);
    rusqlite::Connection::open(dir.path().join("swarm.sqlite"))
        .unwrap()
        .execute("UPDATE members SET status='dead' WHERE id='worker'", [])
        .unwrap();
    let answer = call(&handles, "parent", "recover", json!([1])).unwrap();
    assert_row(
        &handles,
        "recover",
        &answer,
        1,
        ("ready", Value::Null, Value::Null),
    );
}

#[test]
fn evidence_answers_the_recorded_evidence() {
    let (_dir, handles) = board(1_000.0);
    claimed(&handles);
    let accepted = call(
        &handles,
        "parent",
        "evidence",
        json!(["t", "ci.log", "R1", "command", true]),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&accepted).unwrap(),
        r#"{"criterion":"t","artifact":"ci.log","revision":"R1","kind":"command","actor":"parent","accepted":true}"#
    );
    // The same record again changes nothing, and answers it again.
    let again = call(
        &handles,
        "parent",
        "evidence",
        json!(["t", "ci.log", "R1", "command", true]),
    )
    .unwrap();
    assert_eq!(again, accepted);
    // A worker's pass is a proposal: recorded, not accepted.
    let proposed = call(
        &handles,
        "worker",
        "evidence",
        json!(["t", "ci.log", "R1", "command", true]),
    )
    .unwrap();
    assert_eq!(
        (&proposed["actor"], &proposed["accepted"]),
        (&json!("worker"), &json!(false)),
        "{proposed}"
    );
}

/// `release_files` answers the task, the reservation, and how many files it
/// released (round-1 review L1): a reservation that holds nothing (already
/// released, or never there) answers `released: 0`, not a silent success.
#[test]
fn release_files_answers_the_task_the_reservation_and_what_it_released() {
    let (_dir, handles) = board(1_000.0);
    let token = claimed(&handles);
    let reserved = call(
        &handles,
        "worker",
        "reserve",
        json!([1, token, ["a.rs", "b.rs"]]),
    )
    .unwrap();
    let reservation = reserved["token"].clone();
    for released in [2, 0] {
        let answer = call(
            &handles,
            "worker",
            "release_files",
            json!([1, token, reservation]),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_string(&answer).unwrap(),
            format!(r#"{{"task_id":1,"reservation":{reservation},"released":{released}}}"#),
        );
    }
    let bogus = call(
        &handles,
        "worker",
        "release_files",
        json!(["1", token, "bogus"]),
    )
    .unwrap();
    assert_eq!(
        bogus,
        json!({"task_id": 1, "reservation": "bogus", "released": 0}),
        "the task id its row holds, the reservation as given"
    );
    assert_eq!(
        call(&handles, "worker", "file_owners", json!([])).unwrap(),
        json!([])
    );
}

/// `evidence` answers the criterion as the evidence row stores it (round-1
/// review nit), not as the caller gave it: a criterion configured as the
/// integer 1 (a run edited outside the board) is stored as the text "1".
#[test]
fn evidence_answers_the_criterion_as_stored() {
    let (dir, handles, _) = board_and_repository(1_000.0);
    running(&handles);
    rusqlite::Connection::open(dir.path().join("swarm.sqlite"))
        .unwrap()
        .execute(
            r#"UPDATE run SET criteria='[{"id": 1, "kind": "command", "description": "d"}]'"#,
            [],
        )
        .unwrap();
    let recorded = call(
        &handles,
        "parent",
        "evidence",
        json!([1, "ci.log", "R1", "command", true]),
    )
    .unwrap();
    assert_eq!(recorded["criterion"], json!("1"), "{recorded}");
    assert_eq!(recorded["accepted"], json!(true), "{recorded}");
}

#[test]
fn ack_and_withdraw_answer_the_message_and_whether_it_changed() {
    let (_dir, handles) = board(1_000.0);
    claimed(&handles);
    call(
        &handles,
        "parent",
        "send",
        json!(["one", "worker", "hello"]),
    )
    .unwrap();
    for changed in [true, false] {
        assert_eq!(
            call(&handles, "worker", "ack", json!([1])).unwrap(),
            json!({"message_id": 1, "changed": changed})
        );
    }
    call(
        &handles,
        "parent",
        "send",
        json!(["two", "worker", "again"]),
    )
    .unwrap();
    for changed in [true, false] {
        assert_eq!(
            call(&handles, "parent", "withdraw", json!([2])).unwrap(),
            json!({"message_id": 2, "changed": changed})
        );
    }
}

#[test]
fn amend_answers_the_contract_as_it_now_stands() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    let criteria = json!([{"id": "t", "kind": "review", "description": "d"}]);
    let answer = call(
        &handles,
        "parent",
        "amend",
        json!({"goal": "new goal", "constraints": ["c"], "criteria": criteria, "reason": "agreed"}),
    )
    .unwrap();
    assert_eq!(
        answer,
        json!({"goal": "new goal", "constraints": ["c"], "criteria": criteria}),
        "{answer}"
    );
}

#[test]
fn complete_answers_the_run_as_it_now_stands() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    call(
        &handles,
        "parent",
        "evidence",
        json!(["t", "ci.log", "R1", "command", true]),
    )
    .unwrap();
    let answer = call(&handles, "parent", "complete", json!(["R1"])).unwrap();
    assert_eq!(
        (&answer["status"], &answer["outcome"], &answer["reason"]),
        (
            &json!("paused"),
            &json!("succeeded"),
            &json!("completed at R1")
        ),
        "{answer}"
    );
    // The run row alone: the full control receipt would read the usage
    // ledger, which creates its tables (the board file would change).
    let keys: Vec<&str> = answer
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["status", "outcome", "reason"], "{answer}");
}
