//! What the loss and death use cases share (#2277, #1924, #1961): the
//! port of `Workbench._lost`, `_grace_elapsed` and `_end_by_loss`, and the
//! constants they record. Capability-internal helpers, not a use case and
//! not a port.
//!
//! A member's liveness asks the board's one allowlist
//! ([`status_is_alive`]): a member whose status is unknown or NULL (only a
//! hand edit writes one) is lost here, where Python's `status == 'dead'`
//! goes on (the `unknown_member_status_is_not_alive` divergence).
use serde_json::Value;

use super::board_control::edited;
use super::board_operation::{detail, end, text};
use super::ports::{BoardEvents, BoardMembers, BoardRuns, Clock};
use crate::domain::swarm::{
    BoardError, RunRecord, RunState, python_equal, python_truthy, status_is_alive,
};

/// `Workbench.LOSS_GRACE`: seconds from a member's first authorised loss
/// observation until its loss may be recorded (#1961).
pub(crate) const LOSS_GRACE: f64 = 10.0;

/// `Workbench.ABRUPT_BLOCKER`: the blocker of a member's work after an
/// abrupt exit, whose reservations are retained.
pub(crate) const ABRUPT_BLOCKER: &str =
    "worker death confirmed (abrupt exit; reservations retained); coordinator recovery required";

/// `Workbench.ORDERLY_BLOCKER`: the blocker of a member's work after an
/// orderly exit.
pub(crate) const ORDERLY_BLOCKER: &str = "worker death confirmed; coordinator recovery required";

/// The reason a `scope_unknown` event records.
pub(crate) const SCOPE_UNKNOWN: &str =
    "harness exited; execution scope unconfirmed; discard environment";

/// The reason a lost harness ends the run with.
pub(crate) const LOST_HARNESS: &str = "harness exited; execution scope unconfirmed";

/// `Workbench._lost(db, member)`: whether `member` has no row, is not
/// alive, or was recorded lost after its latest activation.
///
/// # Errors
/// The store's (a member it cannot bind included).
pub(crate) fn lost(
    transaction: &(impl BoardMembers + ?Sized),
    member: &Value,
) -> Result<bool, BoardError> {
    let alive = transaction
        .member_status(member)?
        .is_some_and(|row| status_is_alive(row.status.as_deref()));
    if !alive {
        return Ok(true);
    }
    transaction.lost_after_activation(member)
}

/// `Workbench._grace_elapsed(db, member)` for `actor`: the loss
/// observations are scanned in id order for `member`'s; the earliest
/// one's time is where the grace runs from, and the scan stops at the
/// actor's own. Without one of its own, the actor's observation is
/// recorded now (and is the earliest when there is none). Whether the
/// grace has elapsed.
///
/// Python's `for … else` is ported with an explicit `found` flag: the
/// `else` branch (the recording) runs only when the loop ends without its
/// `break`.
///
/// # Errors
/// `the board's loss observation is not as the board writes it` for an
/// earliest time that is not a number (the `outside_edited_loss_records`
/// divergence), or the store's.
pub(crate) fn grace_elapsed(
    transaction: &(impl BoardEvents + ?Sized),
    clock: &dyn Clock,
    actor: &str,
    member: &Value,
) -> Result<bool, BoardError> {
    let now = clock.now_seconds();
    let mut first: Option<Value> = None;
    let mut found = false;
    for observation in transaction.scope_observations()? {
        if !python_equal(&observation.member, member) {
            continue;
        }
        // Python's `if first is None`: a NULL time leaves it unset for
        // the next observation of the member.
        if first.as_ref().is_none_or(Value::is_null) {
            first = Some(observation.time);
        }
        if observation.actor.as_deref() == Some(actor) {
            found = true;
            break;
        }
    }
    if !found {
        transaction.event(
            actor,
            clock.now_seconds(),
            "scope_observed",
            &detail([("member", member.clone())]),
        )?;
        if first.as_ref().is_none_or(Value::is_null) {
            first = Some(Value::from(now));
        }
    }
    let first = first
        .as_ref()
        .and_then(Value::as_f64)
        .ok_or_else(|| edited("loss observation"))?;
    Ok(now - first >= LOSS_GRACE)
}

/// `Workbench._end_by_loss(db, run, reason)` for `actor`: a lost harness
/// fails the setup placeholder, ends a running run as a pause holding
/// `failed` (#1729), and gives an outcome-less pause `failed` while it
/// keeps the pause's start (the frozen budget); a run that holds an
/// outcome or has ended is left as it is. Whether the run changed.
///
/// # Errors
/// The store's.
pub(crate) fn end_by_loss(
    transaction: &(impl BoardRuns + BoardEvents + ?Sized),
    clock: &dyn Clock,
    actor: &str,
    run: &RunRecord,
    reason: &str,
) -> Result<bool, BoardError> {
    let stopped = || {
        transaction.event(
            actor,
            clock.now_seconds(),
            "stop",
            &detail([("status", text("failed")), ("reason", text(reason))]),
        )
    };
    match run.status.as_ref().map(RunState::as_str) {
        Some("setup") => {
            transaction.set_outcome(&RunState::FAILED)?;
            stopped()?;
            Ok(true)
        }
        Some("running") => {
            end(transaction, clock, actor, "failed", reason)?;
            Ok(true)
        }
        Some("paused") if !holds_outcome(run) => {
            transaction.hold_failed(reason)?;
            stopped()?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Python's truth of `run.get('outcome')`.
pub(crate) fn holds_outcome(run: &RunRecord) -> bool {
    run.outcome
        .as_deref()
        .is_some_and(|outcome| python_truthy(&text(outcome)))
}

#[cfg(test)]
#[path = "board_loss_tests.rs"]
mod tests;
