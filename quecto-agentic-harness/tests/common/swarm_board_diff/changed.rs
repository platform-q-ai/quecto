//! The answers the Rust board deliberately changed after the goldens were
//! frozen (#2394): where Python answered `None`, an op that changes a task
//! answers the task's row as it now stands, `evidence` the evidence as
//! recorded, `release_files` `{task_id, reservation, released}`,
//! `withdraw` and `ack` `{message_id, changed}`, `amend` the contract and
//! `complete` the run's `{status, outcome, reason}`.
//!
//! The fixtures stay frozen (the Python board that recorded them is gone,
//! and the MANIFEST only shrinks), so a golden step of one of these
//! methods still holds Python's `null`. The comparison accepts, for such a
//! step alone, the changed answer, checked against the step's own
//! arguments and the Rust board's file (round-1 review M1): the answer
//! must name the task, message or reservation the arguments bind to, as
//! SQLite binds them (`"1"` and `true` find task 1), and every field must
//! be the one the board holds after the step. Every other answer, every
//! refusal and every board file still compares exactly, so the board's
//! state is guarded as before.
use std::path::Path;

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;

use super::Outcome;
use super::scenario::Step;

/// The methods whose `null` answer #2394 changed, with their parameters
/// in the signature's order (an allowlist: any other method's golden
/// `null` still compares exactly).
pub const CHANGED_ANSWERS: [(&str, &[&str]); 14] = [
    ("dependencies", &["task_id", "dependencies"]),
    ("release", &["task_id", "token"]),
    ("block", &["task_id", "token", "reason"]),
    ("unblock", &["task_id", "token", "reason"]),
    ("submit", &["task_id", "token", "evidence"]),
    ("verify_task", &["task_id", "token", "revision"]),
    ("revalidate_task", &["task_id", "revision", "evidence"]),
    ("recover", &["task_id", "release_files"]),
    (
        "evidence",
        &["criterion", "artifact", "revision", "kind", "passed"],
    ),
    ("release_files", &["task_id", "token", "reservation"]),
    ("withdraw", &["message_id"]),
    ("ack", &["message_id"]),
    ("amend", &["goal", "constraints", "criteria", "reason"]),
    ("complete", &["revision"]),
];

/// The task columns the board loads from their JSON text.
const JSON_COLUMNS: [&str; 3] = ["acceptance", "dependencies", "evidence"];

/// What the board held before a step that a changed answer reports on:
/// for `release_files`, how many files the reservation held.
#[derive(Clone, Copy, Debug, Default)]
pub struct Before {
    held_files: Option<i64>,
}

/// One changed-answer step: its method's parameters bound to its
/// arguments, its member, and the board after it.
struct Checked<'a> {
    parameters: &'a [&'a str],
    arguments: Value,
    member: &'a str,
    board: Connection,
}

impl Checked<'_> {
    /// The argument bound to `name`, positionally or by name (`None` for a
    /// default, which the board fills).
    fn argument(&self, name: &str) -> Option<&Value> {
        match &self.arguments {
            Value::Array(values) => self
                .parameters
                .iter()
                .position(|parameter| *parameter == name)
                .and_then(|at| values.get(at)),
            Value::Object(fields) => fields.get(name),
            _ => None,
        }
    }

    /// The id of the `table` row the argument `name` binds to, as SQLite
    /// compares it with the INTEGER PRIMARY KEY.
    fn bound_id(&self, table: &str, name: &str) -> Result<i64, String> {
        let argument = self
            .argument(name)
            .ok_or_else(|| format!("no {name} argument"))?;
        self.board
            .query_row(
                &format!("SELECT id FROM {table} WHERE id=?"),
                params![bound(argument)],
                |row| row.get(0),
            )
            .map_err(|error| format!("no {table} row the {name} argument binds to: {error}"))
    }
}

fn parameters(method: &str) -> Option<&'static [&'static str]> {
    CHANGED_ANSWERS
        .iter()
        .find(|(name, _)| *name == method)
        .map(|(_, parameters)| *parameters)
}

fn open(database: &Path) -> Result<Connection, String> {
    Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("open the Rust board: {error}"))
}

fn arguments_of(step: &Step) -> Value {
    super::golden::decoded(&step.args)
}

/// What a changed answer of `step` will be checked against that the step
/// itself changes: read before the step runs (nothing for other steps).
pub fn before(step: &Step, database: &Path) -> Before {
    let (Some(parameters), true) = (
        parameters(&step.method).filter(|_| step.method == "release_files"),
        database.exists(),
    ) else {
        return Before::default();
    };
    let Ok(board) = open(database) else {
        return Before::default();
    };
    let checked = Checked {
        parameters,
        arguments: arguments_of(step),
        member: &step.member,
        board,
    };
    let held = |checked: &Checked<'_>| -> Option<i64> {
        let [task, claim, token] = ["task_id", "token", "reservation"]
            .map(|name| checked.argument(name).map(bound).unwrap_or(SqlValue::Null));
        checked
            .board
            .query_row(
                "SELECT count(*) FROM files WHERE task=? AND owner=? AND claim=? AND token=?",
                params![task, checked.member, claim, token],
                |row| row.get(0),
            )
            .ok()
    };
    Before {
        held_files: held(&checked),
    }
}

/// For a step whose golden answer is Python's `null`, of a method in
/// [`CHANGED_ANSWERS`]: whether the Rust board's answer is the one #2394
/// gives instead, as the step's arguments and `database` (the Rust board's
/// file after the step) say (a `null` from the Rust board is no longer
/// one). `None` for any other step, which compares exactly.
pub fn superseded(
    step: &Step,
    golden: &Outcome,
    rust: &Outcome,
    database: &Path,
    before: Before,
) -> Option<Result<(), String>> {
    let method = step.method.as_str();
    match (golden, rust, parameters(method)) {
        (Outcome::Ok(Value::Null), Outcome::Ok(answer), Some(parameters)) => Some(
            checked(step, parameters, answer, database, before)
                .map_err(|problem| format!("#2394's {method} answer: {problem} (answer {answer})")),
        ),
        _ => None,
    }
}

fn checked(
    step: &Step,
    parameters: &[&str],
    answer: &Value,
    database: &Path,
    before: Before,
) -> Result<(), String> {
    let checked = Checked {
        parameters,
        arguments: arguments_of(step),
        member: &step.member,
        board: open(database)?,
    };
    match step.method.as_str() {
        "evidence" => evidence(&checked, answer),
        "release_files" => released_files(&checked, answer, before),
        "withdraw" => settled(&checked, answer, Settles::Withdraw),
        "ack" => settled(&checked, answer, Settles::Ack),
        "amend" => contract(&checked, answer),
        "complete" => ended_run(&checked, answer),
        "dependencies" | "release" | "block" | "unblock" | "submit" | "verify_task"
        | "revalidate_task" | "recover" => task_row(&checked, answer),
        other => Err(format!("{other} has no changed-answer check")),
    }
}

/// The keys of `answer`, an object, in order.
fn keys(answer: &Value) -> Result<Vec<&str>, String> {
    answer
        .as_object()
        .map(|fields| fields.keys().map(String::as_str).collect())
        .ok_or_else(|| "not an object".to_owned())
}

/// A JSON argument as Python's `sqlite3` binds it.
fn bound(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Bool(flag) => SqlValue::Integer(i64::from(*flag)),
        Value::Number(number) => number
            .as_i64()
            .map(SqlValue::Integer)
            .or_else(|| number.as_f64().map(SqlValue::Real))
            .unwrap_or(SqlValue::Null),
        Value::String(text) => SqlValue::Text(text.clone()),
        other => SqlValue::Text(other.to_string()),
    }
}

/// A stored cell as the board reads it.
fn json_of(value: SqlValue) -> Value {
    match value {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(integer) => Value::from(integer),
        SqlValue::Real(real) => Value::from(real),
        SqlValue::Text(text) => Value::from(text),
        SqlValue::Blob(bytes) => Value::from(format!("<blob {} bytes>", bytes.len())),
    }
}

/// A stored JSON column as the board loads it.
fn loaded(column: &str, value: SqlValue) -> Result<Value, String> {
    match value {
        SqlValue::Text(text) => {
            quecto::infrastructure::persistence::swarm_board::py_json::decode(&text)
                .and_then(|value| value.to_value())
                .map_err(|error| format!("column {column} holds no JSON: {error}"))
        }
        other => Ok(json_of(other)),
    }
}

/// `column` of the answer is `stored`.
fn same(answer: &Value, column: &str, stored: &Value) -> Result<(), String> {
    match answer.get(column) == Some(stored) {
        true => Ok(()),
        false => Err(format!(
            "column {column} is not the board's: {} against {stored}",
            answer.get(column).unwrap_or(&Value::Null)
        )),
    }
}

/// One row of `sql` (bound to `parameters`) as `(column, value)` pairs in
/// the table's order, each JSON column loaded.
fn stored_row(
    board: &Connection,
    sql: &str,
    parameters: impl rusqlite::Params,
    json_columns: &[&str],
) -> Result<Vec<(String, Value)>, String> {
    let mut statement = board.prepare(sql).map_err(|error| error.to_string())?;
    let names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let cells: Vec<SqlValue> = statement
        .query_row(parameters, |row| {
            (0..names.len())
                .map(|at| row.get::<_, SqlValue>(at))
                .collect()
        })
        .map_err(|error| format!("the board holds no such row: {error}"))?;
    names
        .into_iter()
        .zip(cells)
        .map(|(name, cell)| {
            let value = match json_columns.contains(&name.as_str()) {
                true => loaded(&name, cell)?,
                false => json_of(cell),
            };
            Ok((name, value))
        })
        .collect()
}

/// The task's dict as it now stands: the task the `task_id` argument binds
/// to, every column in the table's order (the JSON columns loaded), a
/// `ready` task whose dependencies are not all completed read `blocked`
/// with the blocker `unmet dependencies`, as `Tasks._task` reads it.
fn task_row(checked: &Checked<'_>, answer: &Value) -> Result<(), String> {
    let id = checked.bound_id("tasks", "task_id")?;
    let mut row = stored_row(
        &checked.board,
        "SELECT * FROM tasks WHERE id=?",
        params![id],
        &JSON_COLUMNS,
    )?;
    let names: Vec<&str> = row.iter().map(|(name, _)| name.as_str()).collect();
    if keys(answer)? != names {
        return Err("not a task's dict in the table's order".to_owned());
    }
    if answer["id"] != id {
        return Err("id is not the task the argument names".to_owned());
    }
    let column = |row: &[(String, Value)], name: &str| {
        row.iter()
            .find(|(column, _)| column == name)
            .map(|(_, value)| value.clone())
            .unwrap_or(Value::Null)
    };
    if column(&row, "status") == "ready" && unmet(&checked.board, &column(&row, "dependencies"))? {
        for (name, value) in &mut row {
            match name.as_str() {
                "status" => *value = Value::from("blocked"),
                "blocker" => *value = Value::from("unmet dependencies"),
                _ => {}
            }
        }
    }
    row.iter()
        .try_for_each(|(name, stored)| same(answer, name, stored))
}

/// Whether a dependency (bound as given) is not a completed task.
fn unmet(board: &Connection, dependencies: &Value) -> Result<bool, String> {
    let Value::Array(dependencies) = dependencies else {
        return Ok(false);
    };
    for dependency in dependencies {
        let status: Option<String> = board
            .query_row(
                "SELECT status FROM tasks WHERE id=?",
                params![bound(dependency)],
                |row| row.get(0),
            )
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                error => Err(error.to_string()),
            })?;
        if status.as_deref() != Some("completed") {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `{criterion, artifact, revision, kind, actor, accepted}`: the member's
/// row for the criterion argument, as the board stores it (`accepted` the
/// stored 0 or 1 as a bool).
fn evidence(checked: &Checked<'_>, answer: &Value) -> Result<(), String> {
    let criterion = checked
        .argument("criterion")
        .ok_or("no criterion argument")?;
    let row = stored_row(
        &checked.board,
        "SELECT criterion, artifact, revision, kind, actor, accepted FROM evidence \
         WHERE criterion=? AND actor=?",
        params![bound(criterion), checked.member],
        &[],
    )
    .map_err(|problem| format!("no evidence row of the argument's criterion: {problem}"))?;
    let names: Vec<&str> = row.iter().map(|(name, _)| name.as_str()).collect();
    if keys(answer)? != names {
        return Err("not the evidence row's columns in order".to_owned());
    }
    row.into_iter().try_for_each(|(name, stored)| {
        let stored = match (name.as_str(), &stored) {
            ("accepted", Value::Number(flag)) => Value::Bool(flag.as_i64() == Some(1)),
            _ => stored,
        };
        same(answer, &name, &stored)
    })
}

/// `{task_id, reservation, released}`: the task the argument binds to, the
/// reservation argument, and how many files it held before the step, none
/// of which it holds now.
fn released_files(checked: &Checked<'_>, answer: &Value, before: Before) -> Result<(), String> {
    if keys(answer)? != ["task_id", "reservation", "released"] {
        return Err("not {task_id, reservation, released}".to_owned());
    }
    let id = checked.bound_id("tasks", "task_id")?;
    if answer["task_id"] != id {
        return Err("task_id is not the task the argument names".to_owned());
    }
    let reservation = checked
        .argument("reservation")
        .ok_or("no reservation argument")?;
    if &answer["reservation"] != reservation {
        return Err("reservation is not the argument".to_owned());
    }
    let held = before
        .held_files
        .ok_or("the files the reservation held were not read before the step")?;
    if answer["released"] != held {
        return Err(format!(
            "released is not the files the reservation held before the step: {held}"
        ));
    }
    let left: i64 = checked
        .board
        .query_row(
            "SELECT count(*) FROM files WHERE task=? AND owner=? AND token=?",
            params![id, checked.member, bound(reservation)],
            |row| row.get(0),
        )
        .map_err(|error| format!("read the files: {error}"))?;
    match left {
        0 => Ok(()),
        _ => Err(format!("the reservation still holds {left} files")),
    }
}

/// Which message op settled the message.
#[derive(Clone, Copy)]
enum Settles {
    Ack,
    Withdraw,
}

/// `{message_id, changed}`: the message the argument binds to, whose
/// stored status is the op's when it changed it, and already settled
/// when it did not (`ack` leaves any message but an unread one, `withdraw`
/// only a withdrawn one).
fn settled(checked: &Checked<'_>, answer: &Value, settles: Settles) -> Result<(), String> {
    if keys(answer)? != ["message_id", "changed"] {
        return Err("not {message_id, changed}".to_owned());
    }
    let id = checked.bound_id("messages", "message_id")?;
    if answer["message_id"] != id {
        return Err("message_id is not the message the argument names".to_owned());
    }
    let changed = answer["changed"].as_bool().ok_or("changed is no bool")?;
    let status: Option<String> = checked
        .board
        .query_row(
            "SELECT status FROM messages WHERE id=?",
            params![id],
            |row| row.get(0),
        )
        .map_err(|error| format!("read the message: {error}"))?;
    let status = status.as_deref();
    let (expected, settled) = match settles {
        Settles::Ack => ("consumed", status != Some("accepted")),
        Settles::Withdraw => ("withdrawn", status == Some("withdrawn")),
    };
    match (changed, status == Some(expected), settled) {
        (true, true, _) | (false, _, true) => Ok(()),
        (true, false, _) => Err(format!("status is not {expected}: {status:?}")),
        (false, _, false) => Err(format!("an unchanged message is not settled: {status:?}")),
    }
}

/// `{goal, constraints, criteria}`: the run's contract as stored, the
/// constraints and criteria loaded from their JSON.
fn contract(checked: &Checked<'_>, answer: &Value) -> Result<(), String> {
    if keys(answer)? != ["goal", "constraints", "criteria"] {
        return Err("not {goal, constraints, criteria}".to_owned());
    }
    stored_row(
        &checked.board,
        "SELECT goal, constraints, criteria FROM run",
        [],
        &["constraints", "criteria"],
    )?
    .iter()
    .try_for_each(|(name, stored)| same(answer, name, stored))
}

/// `{status, outcome, reason}`: the ended run's status, outcome and
/// `outcome_reason` as stored.
fn ended_run(checked: &Checked<'_>, answer: &Value) -> Result<(), String> {
    if keys(answer)? != ["status", "outcome", "reason"] {
        return Err("not {status, outcome, reason}".to_owned());
    }
    stored_row(
        &checked.board,
        "SELECT status, outcome, outcome_reason AS reason FROM run",
        [],
        &[],
    )?
    .iter()
    .try_for_each(|(name, stored)| same(answer, name, stored))
}
