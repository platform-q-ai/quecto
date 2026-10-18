//! The harness's own reads of the board dispatch: `_status`,
//! `_event_cursor`, `_snapshot`, (#2313 review M1) `_run_totals`, the
//! run's totals the coordinator's harness reads when the run settles, for
//! the run-wide section of its `swarm_run_summary`, and (#2338) `_watch`,
//! the run watch's tick. Each but `_watch` answers `read`; none names a
//! task or message. `_board` and `_board_revision` (#2478) are the
//! supervising session's projection of the whole board and its revision,
//! membership-free.
use serde_json::{Map, Value};

use super::method::Parameter;
use super::{Served, float, object, snapshot, status, take};
use crate::application::swarm::dto::{BoardProjection, RunTotalsView};
use crate::application::swarm::use_cases::{
    ReadBoardProjection, ReadEventCursor, ReadRunSnapshot, ReadRunStatus, ReadRunTotals,
};
use crate::domain::swarm::events::watch_polls::{SNAPSHOT, UNCHANGED};
use crate::domain::swarm::{
    events::telemetry::{BoardOpDetail, RefusalKind},
    value_objects::board_error::BoardError,
};

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

pub(super) fn board(read_board_projection: &ReadBoardProjection) -> Result<Served, BoardError> {
    Ok(read(projection(read_board_projection.execute()?)))
}

/// `_board_revision` (#2478): `{run, revision, created}`, the board's
/// revision, the id of the run it counts (null before a run row exists)
/// and whether that run was created, which a supervising session polls to
/// learn the board moved or was made again.
pub(super) fn revision(read_board_projection: &ReadBoardProjection) -> Result<Served, BoardError> {
    let revision = read_board_projection.revision()?;
    Ok(read(object([
        ("run", text(revision.run)),
        ("revision", Value::from(revision.revision)),
        ("created", Value::Bool(revision.created)),
    ])))
}

fn text(value: Option<String>) -> Value {
    value.map_or(Value::Null, Value::String)
}

/// `_board`'s answer, the shape the parent socket's `get_swarm_board`
/// will answer: the board's revision, the run (`null` before
/// one is created), its items, members, criterion evidence and totals, and
/// whether rows were left out.
pub(super) fn projection(board: BoardProjection) -> Value {
    let run = board.run.map_or(Value::Null, |run| {
        let budget = run.budget;
        object([
            ("id", Value::String(run.id)),
            ("goal", Value::String(run.goal)),
            ("status", Value::from(run.status.as_str())),
            ("outcome", text(run.outcome)),
            ("outcome_reason", text(run.outcome_reason)),
            ("generation", Value::from(run.generation)),
            ("criteria", run.criteria),
            ("member_limit", Value::from(run.member_limit)),
            ("deadline", float(run.deadline).unwrap_or(Value::Null)),
            ("coordinator", text(run.coordinator)),
            ("integrator", text(run.integrator)),
            (
                "budget",
                object([
                    (
                        "token_limit",
                        budget.token_limit.map_or(Value::Null, Value::from),
                    ),
                    ("strict_unknown", Value::Bool(budget.strict_unknown)),
                    ("warned", Value::Bool(budget.warned)),
                    ("observed_tokens", Value::from(budget.observed_tokens)),
                    (
                        "unknown_usage_requests",
                        Value::from(budget.unknown_usage_requests),
                    ),
                ]),
            ),
        ])
    });
    let strings = |texts: Vec<String>| Value::Array(texts.into_iter().map(Value::String).collect());
    let items = board.items.into_iter().map(|item| {
        object([
            ("id", Value::from(item.id)),
            ("title", Value::String(item.title)),
            ("criteria", strings(item.criteria)),
            (
                "depends_on",
                Value::Array(item.depends_on.into_iter().map(Value::from).collect()),
            ),
            ("state", Value::from(item.state.as_str())),
            ("waiting", Value::Bool(item.waiting)),
            ("owner", text(item.owner)),
            ("blocker", text(item.blocker)),
            ("reserved_paths", strings(item.reserved_paths)),
            ("evidence", item.evidence),
        ])
    });
    let members = board.members.into_iter().map(|member| {
        object([
            ("id", Value::String(member.id)),
            ("role", Value::from(member.role.as_str())),
            ("status", Value::from(member.status.as_str())),
            (
                "claims",
                Value::Array(member.claims.into_iter().map(Value::from).collect()),
            ),
            (
                "last_activity",
                member
                    .last_activity
                    .and_then(|at| float(at).ok())
                    .unwrap_or(Value::Null),
            ),
        ])
    });
    let evidence = board.evidence.into_iter().map(|row| {
        object([
            ("criterion", Value::String(row.criterion)),
            ("artifact", text(row.artifact)),
            ("revision", text(row.revision)),
            ("kind", text(row.kind)),
            ("actor", Value::String(row.actor)),
            ("accepted", Value::Bool(row.accepted)),
        ])
    });
    let totals = board.totals;
    object([
        ("revision", Value::from(board.revision)),
        ("run", run),
        ("items", Value::Array(items.collect())),
        ("members", Value::Array(members.collect())),
        ("evidence", Value::Array(evidence.collect())),
        (
            "totals",
            object([
                ("items", Value::from(totals.items)),
                ("ready", Value::from(totals.ready)),
                ("waiting", Value::from(totals.waiting)),
                ("claimed", Value::from(totals.claimed)),
                ("blocked", Value::from(totals.blocked)),
                ("submitted", Value::from(totals.submitted)),
                ("completed", Value::from(totals.completed)),
                ("members", Value::from(totals.members)),
                (
                    "members_without_claim",
                    Value::from(totals.members_without_claim),
                ),
                ("members_dead", Value::from(totals.members_dead)),
            ]),
        ),
        ("truncated", Value::Bool(board.truncated)),
    ])
}
