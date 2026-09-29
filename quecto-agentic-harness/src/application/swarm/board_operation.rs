//! The board's operation gate (#2270): the port of Python's
//! `Coordination.operation` and `Coordination._end`, the capability-internal
//! helper every membership-bound board use case runs through. Not a use
//! case and not a port.
//!
//! An operation runs **two** transactions. The first authorises read-only
//! and, when the run's deadline has come, ends it as a pause holding
//! `budget-exhausted` (reason `deadline`) and commits, so the expiry is
//! recorded even when the operation itself is then refused. The second
//! authorises with the operation's own [`Access`], requires the budget when
//! the operation needs a running run, and runs the operation's work.
use serde_json::{Map, Value};

use super::ports::{BoardEvents, BoardRepository, BoardRuns, BoardTransaction, Clock};
use crate::domain::swarm::{
    Access, BoardError, RefusalKind, RunRecord, authorize, expired, require_budget,
};

/// Runs `work` once inside one board transaction and returns its value.
///
/// # Errors
/// `work`'s refusal after the rollback, or the store's.
pub(crate) fn atomic<T>(
    repository: &dyn BoardRepository,
    create: bool,
    work: impl FnOnce(&dyn BoardTransaction) -> Result<T, BoardError>,
) -> Result<T, BoardError> {
    let mut work = Some(work);
    let mut value = None;
    repository.atomic(create, &mut |transaction| {
        let work = work.take().ok_or_else(|| {
            BoardError::new(
                RefusalKind::Internal,
                "coordination store ran a transaction's work twice",
            )
        })?;
        value = Some(work(transaction)?);
        Ok(())
    })?;
    // A store that committed without running the work is refused rather
    // than trusted: nothing the caller needed was read or written.
    value.ok_or_else(|| {
        BoardError::new(
            RefusalKind::Internal,
            "coordination store committed without running its work",
        )
    })
}

/// `Coordination.operation(active, coordinator, read_only)` for `actor`:
/// `work` runs in the second transaction with the run it was authorised on.
///
/// # Errors
/// An authorisation or budget refusal, `work`'s refusal, or the store's.
pub(crate) fn operation<T>(
    repository: &dyn BoardRepository,
    clock: &dyn Clock,
    actor: &str,
    access: Access,
    work: impl FnOnce(&dyn BoardTransaction, &RunRecord) -> Result<T, BoardError>,
) -> Result<T, BoardError> {
    // Expiry must commit even when the following mutation is rejected.
    atomic(repository, false, |transaction| {
        let run = transaction.run()?;
        let member = transaction.member(actor)?;
        let reading = Access {
            read_only: true,
            ..Access::default()
        };
        authorize(run.as_ref(), actor, member.as_ref(), reading)?;
        match run {
            Some(run) if expired(&run, clock.now_seconds()) => {
                end(transaction, clock, actor, "budget-exhausted", "deadline")
            }
            _ => Ok(()),
        }
    })?;
    atomic(repository, false, |transaction| {
        let run = transaction.run()?;
        let member = transaction.member(actor)?;
        authorize(run.as_ref(), actor, member.as_ref(), access)?;
        let Some(run) = run else {
            // `authorize` refuses a missing run first.
            return Err(BoardError::new(
                RefusalKind::RunMissing,
                "coordination run missing",
            ));
        };
        if access.active {
            require_budget(&run, clock.now_seconds())?;
        }
        work(transaction, &run)
    })
}

/// `Coordination._end`: the run pauses holding `outcome` (#1729), recorded
/// as a `stop` and then a `paused` event whose `started` is the clock's.
///
/// # Errors
/// The store's refusal.
pub(crate) fn end(
    transaction: &(impl BoardRuns + BoardEvents + ?Sized),
    clock: &dyn Clock,
    actor: &str,
    outcome: &str,
    reason: &str,
) -> Result<(), BoardError> {
    transaction.propose_outcome(outcome, reason)?;
    transaction.event(
        actor,
        clock.now_seconds(),
        "stop",
        &detail([("status", text(outcome)), ("reason", text(reason))]),
    )?;
    let started = seconds(clock.now_seconds());
    transaction.event(
        actor,
        clock.now_seconds(),
        "paused",
        &detail([
            ("reason", text(reason)),
            ("started", started),
            ("outcome", text(outcome)),
        ]),
    )
}

/// An event detail object with `entries` in order.
pub(crate) fn detail<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<Map<String, Value>>(),
    )
}

pub(crate) fn text(value: &str) -> Value {
    Value::String(value.to_owned())
}

/// A clock reading as a JSON float, as Python's `time.time()` is.
pub(crate) fn seconds(now: f64) -> Value {
    debug_assert!(now.is_finite(), "the clock reads finite seconds");
    Value::from(now)
}

#[cfg(test)]
#[path = "board_operation_tests.rs"]
mod tests;
