//! The harness's own reads of the board dispatch: `_status`,
//! `_event_cursor`, `_snapshot` and (#2313 review M1) `_run_totals`, the
//! run's totals the coordinator's harness reads when the run settles, for
//! the run-wide section of its `swarm_run_summary`. Each answers `read`,
//! and names no task or message.
use serde_json::{Map, Value};

use super::{Served, float, object, snapshot, status};
use crate::application::swarm::dto::RunTotalsView;
use crate::application::swarm::use_cases::{
    ReadEventCursor, ReadRunSnapshot, ReadRunStatus, ReadRunTotals,
};
use crate::domain::swarm::{BoardError, BoardOpDetail};

/// A read's answer.
fn read(value: Value) -> Served {
    Served {
        value,
        decision: "read",
        task_id: None,
        message_id: None,
        cursor_moved: None,
        detail: BoardOpDetail::NONE,
        refused: None,
    }
}

pub(super) fn run_status(read_run_status: &ReadRunStatus) -> Result<Served, BoardError> {
    Ok(read(status(read_run_status.execute()?)))
}

pub(super) fn event_cursor(read_event_cursor: &ReadEventCursor) -> Result<Served, BoardError> {
    Ok(read(Value::from(read_event_cursor.execute()?)))
}

pub(super) fn run_snapshot(
    read_run_snapshot: &ReadRunSnapshot,
    member: &str,
) -> Result<Served, BoardError> {
    Ok(read(snapshot(read_run_snapshot.execute(member)?)?))
}

pub(super) fn run_totals(
    read_run_totals: &ReadRunTotals,
    member: &str,
) -> Result<Served, BoardError> {
    Ok(read(totals(read_run_totals.execute(member)?)?))
}

/// `_run_totals`'s answer: counts, each member's usage row as
/// `usage_report` aggregates it, and the two clock readings.
fn totals(view: RunTotalsView) -> Result<Value, BoardError> {
    let counts = view.counts;
    Ok(object([
        ("run_id", view.run_id.map_or(Value::Null, Value::String)),
        (
            "tasks",
            object([
                ("total", Value::from(view.task_count)),
                ("ready", Value::from(counts.ready)),
                ("claimed", Value::from(counts.claimed)),
                ("blocked", Value::from(counts.blocked)),
                ("submitted", Value::from(counts.submitted)),
                ("completed", Value::from(counts.completed)),
            ]),
        ),
        (
            "messages",
            object([
                ("sent", Value::from(view.messages.sent)),
                ("acked", Value::from(view.messages.consumed)),
                ("withdrawn", Value::from(view.messages.withdrawn)),
            ]),
        ),
        (
            "usage",
            Value::Array(
                view.usage
                    .into_iter()
                    .map(|row| Value::Object(row.columns.into_iter().collect::<Map<_, _>>()))
                    .collect(),
            ),
        ),
        // A creation time the board holds as no finite number is none.
        (
            "created_at",
            view.created_at
                .map_or(Value::Null, |at| float(at).unwrap_or(Value::Null)),
        ),
        ("read_at", float(view.read_at)?),
    ]))
}
