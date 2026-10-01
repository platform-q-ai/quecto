//! The harness's own reads of the board dispatch: `_status`,
//! `_event_cursor`, `_snapshot`, (#2313 review M1) `_run_totals`, the
//! run's totals the coordinator's harness reads when the run settles, for
//! the run-wide section of its `swarm_run_summary`, and (#2338) `_watch`,
//! the run watch's tick. Each but `_watch` answers `read`; none names a
//! task or message.
use serde_json::{Map, Value};

use super::method::Parameter;
use super::{Served, float, object, snapshot, status, take};
use crate::application::swarm::dto::RunTotalsView;
use crate::application::swarm::use_cases::{
    ReadEventCursor, ReadRunSnapshot, ReadRunStatus, ReadRunTotals,
};
use crate::domain::swarm::watch_polls::{SNAPSHOT, UNCHANGED};
use crate::domain::swarm::{BoardError, BoardOpDetail, RefusalKind};

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
        controls_run: false,
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

/// `_watch(since=None)` (#2338): Rust-only, the run watch's one call a
/// tick.
pub(super) const WATCH: [Parameter; 1] = [Parameter {
    name: "since",
    default: Some(|| Value::Null),
}];

/// `{event_cursor, unchanged: true}` while the board's event cursor is
/// `since`, else `{event_cursor, snapshot}` (the snapshot as `_snapshot`
/// answers it); decided `unchanged` or `snapshot`. `since` is `null` (the
/// snapshot whatever the cursor) or a cursor the board answered: a
/// nonnegative integer.
pub(super) fn run_watch(
    read_run_snapshot: &ReadRunSnapshot,
    member: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [since] = take(arguments)?;
    let since = match since {
        Value::Null => None,
        given => Some(given.as_i64().filter(|since| *since >= 0).ok_or_else(|| {
            BoardError::new(
                RefusalKind::Invalid,
                "since must be an event cursor (a nonnegative integer) or null",
            )
        })?),
    };
    let watched = read_run_snapshot.watch(member, since)?;
    let cursor = ("event_cursor", Value::from(watched.event_cursor));
    let (value, decision) = match watched.snapshot {
        None => (
            object([cursor, ("unchanged", Value::Bool(true))]),
            UNCHANGED,
        ),
        Some(view) => (object([cursor, ("snapshot", snapshot(view)?)]), SNAPSHOT),
    };
    let mut served = read(value);
    served.decision = decision;
    served.cursor_moved = since.map(|_| decision == SNAPSHOT);
    Ok(served)
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
