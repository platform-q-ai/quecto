//! The board dispatch's renderings shared by its method modules (#2270,
//! #2277): `_status`'s and `_snapshot`'s dicts, a member row as
//! `dict(row)`, an object in key order, and a stored REAL as Python's
//! `json` writes it.
use serde_json::{Map, Value};

use crate::application::swarm::dto::{MemberRow, RunSnapshotView, RunStatusView};
use crate::domain::swarm::{BoardError, RefusalKind};

/// `_status`'s dict: the counts, then the run's fields as stored.
pub(super) fn status(view: RunStatusView) -> Value {
    let text = |value: Option<String>| value.map_or(Value::Null, Value::String);
    object([
        (
            "members_without_claim",
            Value::from(view.counts.members_without_claim),
        ),
        ("members_dead", Value::from(view.counts.members_dead)),
        ("id", text(view.id)),
        ("status", text(view.status)),
        ("deadline", view.deadline),
        ("coordinator", text(view.coordinator)),
        ("outcome", text(view.outcome)),
    ])
}

/// `_snapshot`'s dict, with each member row as `dict(row)`.
pub(super) fn snapshot(view: RunSnapshotView) -> Result<Value, BoardError> {
    Ok(object([
        ("status", view.status.map_or(Value::Null, Value::String)),
        (
            "coordinator",
            view.coordinator.map_or(Value::Null, Value::String),
        ),
        ("outcome", view.outcome.map_or(Value::Null, Value::String)),
        ("control_generation", Value::from(view.control_generation)),
        ("deadline", float(view.deadline)?),
        (
            "members",
            Value::Array(view.members.into_iter().map(member_row).collect()),
        ),
    ]))
}

/// `dict(row)`: every column, in table order.
pub(super) fn member_row(row: MemberRow) -> Value {
    Value::Object(row.columns.into_iter().collect())
}

pub(super) fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<Map<String, Value>>(),
    )
}

/// A stored REAL as Python's `json` writes a float. SQLite can hold an
/// infinity that JSON cannot; that is refused rather than written as null.
pub(super) fn float(value: f64) -> Result<Value, BoardError> {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| {
            BoardError::new(
                RefusalKind::Store,
                format!("the board holds a non-finite number: {value}"),
            )
        })
}
