//! A status move and `done_without_run` as both front ends take them
//! (#2705 T4): a move's `reason` is the one given or, given none, what its
//! front end's policy makes of none; `done_without_run` requires one.
use super::{Read, RequestError};
use crate::application::project_board::dto::BoardOp;
use crate::application::project_board::operations::BoardRequest;
use crate::domain::project_board::entities::task_parts::EntryPolicy;
use crate::domain::project_board::services::transitions::MoveReason;

/// The move the request names: its status, and its reason.
///
/// # Errors
/// A status or reason of the wrong type.
pub(super) fn moved(read: &Read<'_>) -> Result<BoardOp, RequestError> {
    let to = read.status("to")?.expect("a required field is present");
    Ok(BoardOp::Move(to, move_reason(read)?))
}

/// The task done without a run, for the reason the request gives.
///
/// # Errors
/// A task id or reason of the wrong type.
pub(super) fn done_without_run(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    let reason = read.text("reason")?.expect("a required field is present");
    Ok(BoardRequest::DoneWithoutRun {
        task: read.task()?,
        reason,
    })
}

/// A move's reason: the one given, or, given none, as its front end's
/// policy lets a move be made without one (an allowlist): the agent tool's
/// is absent, so back to ready from review is refused without one; the
/// CLI's is unasked, as the Pocket relay moves a task back through it with
/// no reason until its task page sends one.
fn move_reason(read: &Read<'_>) -> Result<MoveReason, RequestError> {
    Ok(match (read.text("reason")?, read.policy) {
        (Some(reason), _) => MoveReason::Given(reason),
        (None, EntryPolicy::Sectioned) => MoveReason::Absent,
        (None, EntryPolicy::SectionedOrLegacy) => MoveReason::Unasked,
    })
}

/// A board-head-pinned exception, with conditional reason fields checked first.
pub(super) fn set_outcome(read: &Read<'_>) -> Result<BoardRequest, RequestError> {
    use crate::domain::project_board::value_objects::telemetry::FaultKind;
    use serde_json::Value;
    let index = read.offset("outcome_index")?.ok_or_else(|| {
        RequestError::of(
            "outcome_index",
            "must be an existing zero-based index",
            FaultKind::WrongType,
        )
    })?;
    let met = read
        .value("met")
        .and_then(Value::as_bool)
        .ok_or_else(|| RequestError::of("met", "must be a boolean", FaultKind::WrongType))?;
    let reason = match (met, read.value("reason")) {
        (false, Some(_)) => read.text("reason")?,
        (false, None) => {
            return Err(RequestError::of(
                "reason",
                "is required when unmet",
                FaultKind::Missing,
            ));
        }
        (true, None) => None,
        (true, Some(_)) => {
            return Err(RequestError::of(
                "reason",
                "is taken only when unmet",
                FaultKind::InvalidValue,
            ));
        }
    };
    let expected_rev = match read.pin()? {
        Some(crate::application::project_board::dto::ReadPin::Rev(rev)) => rev,
        _ => {
            return Err(RequestError::of(
                "expected_rev",
                "is required",
                FaultKind::Missing,
            ));
        }
    };
    Ok(BoardRequest::Change(
        read.task()?,
        BoardOp::SetOutcome {
            index,
            reason,
            expected_rev,
        },
    ))
}
