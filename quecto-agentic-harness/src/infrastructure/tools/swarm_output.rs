//! Result helpers for the swarm tool (split from `swarm.rs` for the 750-line
//! cap), and how its answers read to a member (#2478 slice B): in
//! task-item words. The board keeps its own names (its stored values, its
//! refusals and the golden fixtures that pin them), so the tool presents
//! them: each rename is an allowlist of the exact keys and texts the board
//! writes, and anything else is answered as the board answered it.
use serde_json::{Map, Value};

use super::swarm_board_dispatch::BoardWire;
use super::swarm_board_ops::advertised_op;
use crate::domain::error::DomainError;
use crate::domain::tool::ToolResult;

pub(super) fn tool_err(content: String) -> Result<ToolResult, DomainError> {
    Ok(ToolResult {
        content,
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    })
}
pub(super) fn ok_json(v: serde_json::Value, is_error: bool) -> Result<ToolResult, DomainError> {
    Ok(ToolResult {
        content: serde_json::to_string_pretty(&v).unwrap(),
        is_error,
        image_blocks: vec![],
        delivery_metadata: None,
    })
}

/// The board's answer, with the lifecycle's notes: added to an answer that
/// is an object, else beside it as `{"result": answer, …}` (only when there
/// are notes, so an answer is otherwise exactly the board method's). A
/// refusal is answered as the board refused it, the notes (when there are
/// any) on the line after it (#2279 review L4: the notes are kept whatever
/// the outcome, as the removed `op=run` kept them). Both are presented in
/// task-item words (#2478 slice B): the answer of `op` (its old name) by
/// [`task_item_answer`], a refusal by [`task_item_refusal`].
pub(super) fn answered(
    wire: &BoardWire,
    op: &str,
    answer: Result<Value, DomainError>,
    notes: Map<String, Value>,
) -> Result<ToolResult, DomainError> {
    let answer = answer
        .map(|value| task_item_answer(op, value))
        .map_err(task_item_error);
    let value = match (answer, notes.is_empty()) {
        (Ok(value), _) => value,
        (Err(refusal), true) => return tool_err(refusal.to_string()),
        (Err(refusal), false) => {
            let written = (wire.write)(&Value::Object(notes))
                .unwrap_or_else(|error| format!("the notes cannot be written: {error}"));
            return tool_err(format!("{refusal}\n{written}"));
        }
    };
    let value = match (value, notes.is_empty()) {
        (value, true) => value,
        (Value::Object(mut fields), false) => {
            fields.extend(notes);
            Value::Object(fields)
        }
        (other, false) => {
            let mut fields = Map::new();
            fields.insert("result".to_owned(), other);
            fields.extend(notes);
            Value::Object(fields)
        }
    };
    match (wire.write)(&value) {
        Ok(content) => Ok(ToolResult {
            content,
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        }),
        Err(error) => tool_err(format!("swarm: the answer cannot be written: {error}")),
    }
}

/// How a structured op's answer is presented to a member: one task's row,
/// a list of task rows, file rows or messages, or `release_files`' answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presented {
    TaskRow,
    TaskRows,
    FileRows,
    Messages,
    ReleasedFiles,
}

/// How each structured op's answer is presented, by the op's old name; a
/// test pins every board op's entry (or its absence: answered as the board
/// answered it), so a new board op needs a decision.
pub(crate) const PRESENTED_OPS: &[(&str, Presented)] = &[
    ("task", Presented::TaskRow),
    ("task_create", Presented::TaskRow),
    ("dependencies", Presented::TaskRow),
    ("claim", Presented::TaskRow),
    ("release", Presented::TaskRow),
    ("block", Presented::TaskRow),
    ("unblock", Presented::TaskRow),
    ("submit", Presented::TaskRow),
    ("verify_task", Presented::TaskRow),
    ("revalidate_task", Presented::TaskRow),
    ("recover", Presented::TaskRow),
    ("revoke", Presented::TaskRow),
    ("tasks", Presented::TaskRows),
    ("file_owners", Presented::FileRows),
    ("inbox", Presented::Messages),
    ("release_files", Presented::ReleasedFiles),
];

/// How the structured op `op` (its old name) is presented, if it is.
pub(crate) fn presented(op: &str) -> Option<Presented> {
    PRESENTED_OPS
        .iter()
        .find(|(name, _)| *name == op)
        .map(|(_, kind)| *kind)
}

/// The answer of the structured op `op` (its old name) as a member reads
/// it ([`PRESENTED_OPS`]): a task row's recovery hint as the coordinator's
/// call, a file row's `task` as `task_item`, `release_files`' `task_id` as
/// `task_item_id`, and the coordinator's revoke notice as a task item's.
pub(crate) fn task_item_answer(op: &str, value: Value) -> Value {
    match presented(op) {
        Some(Presented::TaskRow) => task_row(value),
        Some(Presented::TaskRows) => rows(value, task_row),
        Some(Presented::FileRows) => rows(value, file_row),
        Some(Presented::Messages) => rows(value, message_row),
        Some(Presented::ReleasedFiles) => renamed(value, "task_id", "task_item_id"),
        None => value,
    }
}

/// The answer of the harness op `op` as a member reads it: a summary's
/// `task_count` and `tasks` as `task_item_count` and `task_items` (each
/// row as [`task_item_answer`] presents it) and its file rows, whichever
/// op answers one (`summary`, `create`, `cancel_run`, a settlement); and
/// `events`' `task_created` and `{"task": N}` as `task_item_created` and
/// `{"task_item": N}`.
pub(crate) fn task_item_harness_answer(op: &str, value: Value) -> Value {
    match (op, value) {
        ("events", Value::Object(page)) => Value::Object(
            page.into_iter()
                .map(|(key, value)| match key.as_str() {
                    "events" => (key, rows(value, event_row)),
                    _ => (key, value),
                })
                .collect(),
        ),
        (_, Value::Object(fields))
            if fields.contains_key("task_count")
                && fields.get("tasks").is_some_and(Value::is_array) =>
        {
            Value::Object(
                fields
                    .into_iter()
                    .map(|(key, value)| match key.as_str() {
                        "task_count" => ("task_item_count".to_owned(), value),
                        "tasks" => ("task_items".to_owned(), rows(value, task_row)),
                        "files" => (key, rows(value, file_row)),
                        _ => (key, value),
                    })
                    .collect(),
            )
        }
        (_, value) => value,
    }
}

/// The board's refusal `message` as a member reads it: the texts the
/// board keeps from Python (the golden fixtures pin them) in task-item
/// words, and a missing `task_id` by its advertised name.
pub(crate) fn task_item_refusal(message: &str) -> String {
    const EXACT: [(&str, &str); 7] = [
        (
            "recovery requires confirmed worker death; revoke(id, reason) reassigns a live owner",
            "recovery requires confirmed worker death; revoke(task_item_id, reason) reassigns a live owner",
        ),
        (
            "reservations retained after an abrupt exit; recover(id, release_files=True) frees them, or revoke(id, reason)",
            "reservations retained after an abrupt exit; recover(task_item_id, release_files=True) frees them, or revoke(task_item_id, reason)",
        ),
        ("unknown task", "unknown task item"),
        (
            "task is not ready to claim",
            "task item is not ready to claim",
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
    ];
    const PREFIXES: [(&str, &str); 2] = [
        ("task board full (", "task item board full ("),
        (
            "task acceptance criteria required: ",
            "task item acceptance criteria required: ",
        ),
    ];
    if let Some((_, member)) = EXACT.iter().find(|(board, _)| *board == message) {
        return (*member).to_owned();
    }
    if let Some((rest, member)) = PREFIXES
        .iter()
        .find_map(|(board, member)| message.strip_prefix(board).map(|rest| (rest, member)))
    {
        return format!("{member}{rest}");
    }
    binding_refusal(message)
}

/// A binding refusal, `<op>: <one of the binding's own texts>`, naming
/// its op as the schema advertises it (`task: missing required argument
/// title` as `task_item: …`); any other message as it is. The op must be a
/// method name (lowercase letters and underscores) and the rest begin with
/// one of the binding's texts (`swarm_board_dispatch_binding::refusal`).
fn binding_refusal(message: &str) -> String {
    const BINDING: [&str; 5] = [
        "missing required argument ",
        "unexpected argument ",
        "takes ",
        "arguments must be a JSON array or object",
        "task_id and task_item_id name the same argument but hold different values; send only one",
    ];
    let Some((op, rest)) = message.split_once(": ") else {
        return message.to_owned();
    };
    let method = !op.is_empty()
        && op
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_');
    // A missing `task_id`, and only that whole text, names the argument
    // as advertised; a member's own key that reads like it is its own.
    const MISSING_ID: (&str, &str) = (
        "missing required argument task_id",
        "missing required argument task_item_id",
    );
    match (method, rest == MISSING_ID.0) {
        (true, true) => format!("{}: {}", advertised_op(op), MISSING_ID.1),
        (true, false) if BINDING.iter().any(|text| rest.starts_with(text)) => {
            format!("{}: {rest}", advertised_op(op))
        }
        (true, false) | (false, _) => message.to_owned(),
    }
}

/// A board refusal, `swarm: "<text>"` (the text as a JSON string), with
/// its text as [`task_item_refusal`] presents it; any other error as it is.
fn task_item_error(error: DomainError) -> DomainError {
    let DomainError::Tool(text) = &error else {
        return error;
    };
    let quoted = text.strip_prefix("swarm: ");
    match quoted.and_then(|quoted| serde_json::from_str::<String>(quoted).ok()) {
        Some(message) => DomainError::Tool(format!(
            "swarm: {}",
            Value::String(task_item_refusal(&message))
        )),
        None => error,
    }
}

/// `value`'s items, each as `row` presents it, when `value` is a list.
fn rows(value: Value, row: fn(Value) -> Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(row).collect()),
        other => other,
    }
}

/// `value` with its key `old` as `new`, in its place, when it is an
/// object holding `old`.
fn renamed(value: Value, old: &str, new: &str) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| match key == old {
                    true => (new.to_owned(), value),
                    false => (key, value),
                })
                .collect::<Map<String, Value>>(),
        ),
        other => other,
    }
}

/// A task row with its recovery hint as the coordinator's call, by the
/// argument the tool advertises (`domain::swarm::owner::owner_recovery`'s
/// three hints).
fn task_row(mut value: Value) -> Value {
    const HINTS: [(&str, &str); 3] = [
        (
            "recover(task) or revoke(task, reason)",
            "recover(task_item_id) or revoke(task_item_id, reason)",
        ),
        (
            "resume the run (agent_cmd swarm_control resume), then revoke(task, reason)",
            "resume the run (agent_cmd swarm_control resume), then revoke(task_item_id, reason)",
        ),
        ("revoke(task, reason)", "revoke(task_item_id, reason)"),
    ];
    if let Some(hint) = value.get_mut("recovery")
        && let Some((_, call)) = HINTS.iter().find(|(board, _)| hint == *board)
    {
        *hint = Value::from(*call);
    }
    value
}

/// A file row, its `task` column as `task_item`.
fn file_row(value: Value) -> Value {
    renamed(value, "task", "task_item")
}

/// An event row: `task_created` as `task_item_created`, and a detail's
/// top-level `task` key (`{..., "task":N, ...}`, as the board writes it)
/// as `task_item`, in its place; a `task` key nested deeper (an evidence
/// entry's) and every other character of the text are as written.
fn event_row(mut value: Value) -> Value {
    if let Some(action) = value.get_mut("action")
        && action == "task_created"
    {
        *action = Value::from("task_item_created");
    }
    if let Some(Value::String(detail)) = value.get_mut("detail")
        && let Some(key) = top_level_key(detail, "task")
    {
        detail.replace_range(key, r#""task_item""#);
    }
    value
}

/// Where `text`, a JSON object, holds the key `name` at its top level: the
/// byte range of its quoted key, found by scanning the text (strings and
/// their escapes skipped, depth counted) so nothing else is read as JSON or
/// rewritten, however deeply it nests. `None` when the text does not begin
/// with `{` (after spaces), a string runs past its end, the object closes
/// before such a key, or there is none.
fn top_level_key(text: &str, name: &str) -> Option<std::ops::Range<usize>> {
    let bytes = text.as_bytes();
    let mut at = bytes.iter().position(|byte| !byte.is_ascii_whitespace())?;
    if bytes[at] != b'{' {
        return None;
    }
    let mut depth = 0usize;
    while at < bytes.len() {
        match bytes[at] {
            b'"' => {
                let start = at;
                at += 1;
                while at < bytes.len() && bytes[at] != b'"' {
                    at += if bytes[at] == b'\\' { 2 } else { 1 };
                }
                if at >= bytes.len() {
                    return None;
                }
                let quoted = start..at + 1;
                let colon = text[at + 1..].trim_start().starts_with(':');
                if depth == 1 && colon && text[quoted.clone()] == format!("\"{name}\"") {
                    return Some(quoted);
                }
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return None;
                }
            }
            _ => {}
        }
        at += 1;
    }
    None
}

/// A message row: the coordinator's revoke notice (`claim on task N
/// revoked by the coordinator: <reason>`, `board_recovery`) as a task
/// item's, when `N` is an id the board writes: letters, digits, `.`, `-`
/// and `+` only (a number, as Python's `str` writes it, or plain text).
/// Who sent it is not checked: the row names its sender, but not whether
/// that member is the run's coordinator, which only a board read would
/// say; the notice's exact shape is what is matched.
fn message_row(mut value: Value) -> Value {
    const NOTICE: (&str, &str) = ("claim on task ", " revoked by the coordinator: ");
    let id_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+');
    if let Some(Value::String(body)) = value.get_mut("body")
        && let Some((id, reason)) = body
            .strip_prefix(NOTICE.0)
            .and_then(|rest| rest.split_once(NOTICE.1))
        && !id.is_empty()
        && id.chars().all(id_char)
    {
        *body = format!("claim on task item {id}{}{reason}", NOTICE.1);
    }
    value
}
