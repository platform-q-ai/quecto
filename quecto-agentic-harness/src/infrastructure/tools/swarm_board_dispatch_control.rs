//! The run-control methods of the board dispatch (#2273): their Python
//! signatures and their serving. `pause`, `stop`, `resume` and the
//! supervisor's `_resume_external`, `_close` and `_extend_deadline` answer
//! the control receipt, as `_control_status` does; `usage_report` answers
//! the report. Every argument reaches the use case as the JSON value
//! passed. The decision names whether the call changed the run or found it
//! already as asked.
use serde_json::Value;

use super::{Parameter, Served, SwarmBoardHandles, object, required, take};
use crate::application::swarm::dto::{
    ControlAnswer, ControlReceipt, ExtendRunDeadlineRequest, PauseRunRequest, RunTransition,
    StopRunRequest, UsageReport, UsageRow,
};
use crate::domain::swarm::BoardError;

/// `pause(reason)`.
pub(super) const PAUSE: [Parameter; 1] = [required("reason")];
/// `stop(status, reason)`.
pub(super) const STOP: [Parameter; 2] = [required("status"), required("reason")];
/// `_extend_deadline(seconds)`.
pub(super) const EXTEND: [Parameter; 1] = [required("seconds")];

/// The receipt, with `applied` as the decision of a change and
/// `unchanged` for a call that found the run already as asked.
fn answered(answer: ControlAnswer, applied: &'static str) -> Served {
    Served {
        value: receipt(answer.receipt),
        decision: match answer.transition {
            RunTransition::Applied => applied,
            RunTransition::Unchanged => "unchanged",
        },
    }
}

pub(super) fn pause(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [reason] = take(arguments)?;
    let answer = handles.pause_run.execute(PauseRunRequest {
        actor: actor.to_owned(),
        reason,
    })?;
    Ok(answered(answer, "paused"))
}

pub(super) fn resume(handles: &SwarmBoardHandles, actor: &str) -> Result<Served, BoardError> {
    let answer = handles.resume_run.execute(actor)?;
    Ok(answered(answer, "resumed"))
}

pub(super) fn resume_external(
    handles: &SwarmBoardHandles,
    actor: &str,
) -> Result<Served, BoardError> {
    let answer = handles.resume_run_externally.execute(actor)?;
    Ok(answered(answer, "resumed"))
}

pub(super) fn close(handles: &SwarmBoardHandles, actor: &str) -> Result<Served, BoardError> {
    let answer = handles.close_run.execute(actor)?;
    Ok(answered(answer, "closed"))
}

pub(super) fn extend(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [seconds] = take(arguments)?;
    let answer = handles
        .extend_run_deadline
        .execute(ExtendRunDeadlineRequest {
            actor: actor.to_owned(),
            seconds,
        })?;
    Ok(answered(answer, "extended"))
}

pub(super) fn stop(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [status, reason] = take(arguments)?;
    let answer = handles.stop_run.execute(StopRunRequest {
        actor: actor.to_owned(),
        status,
        reason,
    })?;
    Ok(answered(answer, "stopped"))
}

pub(super) fn control_status(
    handles: &SwarmBoardHandles,
    actor: &str,
) -> Result<Served, BoardError> {
    Ok(Served {
        value: receipt(handles.read_control_status.execute(actor)?),
        decision: "read",
    })
}

pub(super) fn usage_report(handles: &SwarmBoardHandles, actor: &str) -> Result<Served, BoardError> {
    Ok(Served {
        value: report(handles.read_usage_report.execute(actor)?),
        decision: "read",
    })
}

/// `Coordination._receipt`'s dict, in Python's key order.
fn receipt(receipt: ControlReceipt) -> Value {
    let text = |value: Option<String>| value.map_or(Value::Null, Value::String);
    object([
        ("status", text(receipt.status)),
        ("outcome", text(receipt.outcome)),
        ("reason", text(receipt.reason)),
        ("generation", Value::from(receipt.generation)),
        ("budget", Value::Object(receipt.budget)),
        (
            "resume_blockers",
            Value::Array(
                receipt
                    .resume_blockers
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        ),
    ])
}

/// `usage_report()`'s dict, each aggregate row as `dict(row)`.
fn report(report: UsageReport) -> Value {
    let row = |row: UsageRow| Value::Object(row.columns.into_iter().collect());
    object([
        ("budget", report.budget),
        ("totals", row(report.totals)),
        (
            "members",
            Value::Array(report.members.into_iter().map(row).collect()),
        ),
        (
            "recent_requests",
            Value::Array(
                report
                    .recent_requests
                    .into_iter()
                    .map(|recent| {
                        object([
                            ("member", recent.member),
                            ("observation", recent.observation),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

#[cfg(test)]
#[path = "swarm_board_dispatch_control_tests.rs"]
mod tests;
