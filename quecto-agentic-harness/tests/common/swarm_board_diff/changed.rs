//! The answers the Rust board deliberately changed after the goldens were
//! frozen (#2394): where Python answered `None`, an op that changes a task
//! answers the task's row as it now stands, `evidence` the evidence as
//! recorded, `release_files` `{task_id, reservation}`, `withdraw` and `ack`
//! `{message_id, changed}`, `amend` the contract and `complete` the run's
//! `{status, outcome, reason}`.
//!
//! The fixtures stay frozen (the Python board that recorded them is gone,
//! and the MANIFEST only shrinks), so a golden step of one of these
//! methods still holds Python's `null`. The comparison accepts, for such a
//! step alone, the changed answer, checked against the Rust board's own
//! file after the step: the row the answer names must be the row the board
//! holds. Every other answer, every refusal and every board file still
//! compares exactly, so the board's state is guarded as before.
use std::path::Path;

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;

use super::Outcome;

/// The methods whose `null` answer #2394 changed (an allowlist: any other
/// method's golden `null` still compares exactly).
pub const CHANGED_ANSWERS: [&str; 14] = [
    "dependencies",
    "release",
    "block",
    "unblock",
    "submit",
    "verify_task",
    "revalidate_task",
    "recover",
    "evidence",
    "release_files",
    "withdraw",
    "ack",
    "amend",
    "complete",
];

/// The `tasks` table's columns, in the order a task's dict keeps them.
const TASK_COLUMNS: [&str; 9] = [
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

/// For a step whose golden answer is Python's `null`, of a method in
/// [`CHANGED_ANSWERS`]: whether the Rust board's answer is the one #2394
/// gives instead, as `database` (the Rust board's file after the step)
/// holds it (a `null` from the Rust board is no longer one). `None` for
/// any other step, which compares exactly.
pub fn superseded(
    method: &str,
    member: &str,
    golden: &Outcome,
    rust: &Outcome,
    database: &Path,
) -> Option<Result<(), String>> {
    match (golden, rust, CHANGED_ANSWERS.contains(&method)) {
        (Outcome::Ok(Value::Null), Outcome::Ok(answer), true) => {
            Some(checked(method, member, answer, database))
        }
        _ => None,
    }
}

fn checked(method: &str, member: &str, answer: &Value, database: &Path) -> Result<(), String> {
    let board = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("open the Rust board: {error}"))?;
    let problem = match method {
        "evidence" => evidence(&board, member, answer),
        "release_files" => released_files(&board, answer),
        "withdraw" => settled(&board, answer, "withdrawn"),
        "ack" => settled(&board, answer, "consumed"),
        "amend" => contract(&board, answer),
        "complete" => receipt(&board, answer),
        _ => task_row(&board, answer),
    };
    problem.map_err(|problem| format!("#2394's {method} answer {answer}: {problem}"))
}

/// The keys of `answer`, an object, in order.
fn keys(answer: &Value) -> Result<Vec<&str>, String> {
    answer
        .as_object()
        .map(|fields| fields.keys().map(String::as_str).collect())
        .ok_or_else(|| "not an object".to_owned())
}

fn json_of(value: SqlValue) -> Value {
    match value {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(integer) => Value::from(integer),
        SqlValue::Real(real) => Value::from(real),
        SqlValue::Text(text) => Value::from(text),
        SqlValue::Blob(bytes) => Value::from(format!("<blob {} bytes>", bytes.len())),
    }
}

/// The task's dict as it now stands: its columns in table order, and each
/// stored scalar as the board holds it (`status` derived `blocked`, with
/// the blocker `unmet dependencies`, for a `ready` task whose dependencies
/// are not all completed).
fn task_row(board: &Connection, answer: &Value) -> Result<(), String> {
    if keys(answer)? != TASK_COLUMNS {
        return Err("not a task's dict in column order".to_owned());
    }
    let id = answer["id"].as_i64().ok_or("the task id is no integer")?;
    let stored = board
        .query_row(
            "SELECT title, status, owner, token, blocker FROM tasks WHERE id=?",
            params![id],
            |row| {
                (0..5)
                    .map(|column| row.get::<_, SqlValue>(column).map(json_of))
                    .collect::<Result<Vec<Value>, _>>()
            },
        )
        .map_err(|error| format!("no task {id} on the board: {error}"))?;
    let [title, status, owner, token, blocker] =
        <[Value; 5]>::try_from(stored).expect("five columns");
    let derived = answer["status"] == "blocked"
        && status == "ready"
        && answer["blocker"] == "unmet dependencies";
    let status_as_held = answer["status"] == status && answer["blocker"] == blocker;
    match (
        answer["title"] == title && answer["owner"] == owner && answer["token"] == token,
        status_as_held || derived,
    ) {
        (true, true) => Ok(()),
        _ => Err(format!(
            "the board holds title {title}, status {status}, owner {owner}, token {token}, \
             blocker {blocker}"
        )),
    }
}

/// `{criterion, artifact, revision, kind, actor, accepted}`: the caller's
/// row, as the board holds it.
fn evidence(board: &Connection, member: &str, answer: &Value) -> Result<(), String> {
    let expected = [
        "criterion",
        "artifact",
        "revision",
        "kind",
        "actor",
        "accepted",
    ];
    if keys(answer)? != expected || answer["actor"] != member {
        return Err(format!("not {member}'s evidence record"));
    }
    let accepted = answer["accepted"].as_bool().ok_or("accepted is no bool")?;
    let found: i64 = board
        .query_row(
            "SELECT count(*) FROM evidence WHERE actor=? AND artifact=? AND revision=? \
             AND kind=? AND accepted=?",
            params![
                member,
                answer["artifact"].as_str(),
                answer["revision"].as_str(),
                answer["kind"].as_str(),
                i64::from(accepted)
            ],
            |row| row.get(0),
        )
        .map_err(|error| format!("read the evidence: {error}"))?;
    match found {
        1 => Ok(()),
        _ => Err(format!("{found} evidence rows hold it")),
    }
}

/// `{task_id, reservation}`: a task the board holds, whose reservation by
/// that token holds no file now.
fn released_files(board: &Connection, answer: &Value) -> Result<(), String> {
    if keys(answer)? != ["task_id", "reservation"] {
        return Err("not {task_id, reservation}".to_owned());
    }
    let id = answer["task_id"]
        .as_i64()
        .ok_or("the task id is no integer")?;
    let (tasks, files): (i64, i64) = board
        .query_row(
            "SELECT (SELECT count(*) FROM tasks WHERE id=?1), \
             (SELECT count(*) FROM files WHERE task=?1 AND token=?2)",
            params![id, json_text(&answer["reservation"])],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("read the files: {error}"))?;
    match (tasks, files) {
        (1, 0) => Ok(()),
        _ => Err(format!("{tasks} tasks, {files} files still reserved")),
    }
}

/// A text value as SQLite compares it; anything else as its JSON.
fn json_text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

/// `{message_id, changed}`: a message the board holds, `status` once the
/// call changed it.
fn settled(board: &Connection, answer: &Value, status: &str) -> Result<(), String> {
    if keys(answer)? != ["message_id", "changed"] {
        return Err("not {message_id, changed}".to_owned());
    }
    let id = answer["message_id"]
        .as_i64()
        .ok_or("the message id is no integer")?;
    let changed = answer["changed"].as_bool().ok_or("changed is no bool")?;
    let stored: Option<String> = board
        .query_row(
            "SELECT status FROM messages WHERE id=?",
            params![id],
            |row| row.get(0),
        )
        .map_err(|error| format!("no message {id} on the board: {error}"))?;
    match (changed, stored.as_deref() == Some(status)) {
        (false, _) | (true, true) => Ok(()),
        (true, false) => Err(format!("message {id} is {stored:?}, not {status}")),
    }
}

/// `{goal, constraints, criteria}`: the run's contract as stored.
fn contract(board: &Connection, answer: &Value) -> Result<(), String> {
    if keys(answer)? != ["goal", "constraints", "criteria"] {
        return Err("not {goal, constraints, criteria}".to_owned());
    }
    let goal: SqlValue = board
        .query_row("SELECT goal FROM run", [], |row| row.get(0))
        .map_err(|error| format!("read the run: {error}"))?;
    match answer["goal"] == json_of(goal.clone()) {
        true => Ok(()),
        false => Err(format!("the run's goal is {goal:?}")),
    }
}

/// `{status, outcome, reason}`: the ended run, its status the board's.
fn receipt(board: &Connection, answer: &Value) -> Result<(), String> {
    if keys(answer)? != ["status", "outcome", "reason"] {
        return Err("not {status, outcome, reason}".to_owned());
    }
    let status: SqlValue = board
        .query_row("SELECT status FROM run", [], |row| row.get(0))
        .map_err(|error| format!("read the run: {error}"))?;
    match (
        answer["status"] == json_of(status.clone()),
        &answer["outcome"],
    ) {
        (true, Value::String(outcome)) if outcome == "succeeded" => Ok(()),
        _ => Err(format!("the run's status is {status:?}")),
    }
}
