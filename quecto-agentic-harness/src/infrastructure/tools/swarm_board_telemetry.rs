//! The board's call records (#2270, #2303): one `tracing` record per call
//! on [`TELEMETRY_TARGET`], and, when the event log is switched on, one
//! `swarm_op` record in it through the application's `BoardOpLog` port,
//! written synchronously on the call's own thread (never through an async
//! runtime). Both carry ids, kinds, durations and sizes only, never
//! argument or board text; the caller's member id is [`Redacted`] and
//! bounded to [`ACTOR_REF_CHARS`] characters, since a call can name any
//! member id, admitted or not. What was not measured, or does not apply,
//! is `None` (`null`), never a zero or a `false`.
#[cfg(test)] // Red: wired in the fix commit.
use std::collections::HashMap;
#[cfg(test)] // Red: wired in the fix commit.
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;

use crate::application::swarm::dto::CallMeasure;
use crate::domain::redaction::Redacted;
use crate::domain::swarm::{BoardOpObservation, BoardOpOutcome, BoardRole, RefusalKind};

/// The `tracing` target of every board call record.
pub const TELEMETRY_TARGET: &str = "quecto::swarm_board";

/// A busy wait this long is worth a warning. It is measured only while
/// the event log is on (owner decision T1), so the warning is raised only
/// then.
const SLOW_LOCK: Duration = Duration::from_millis(250);

/// The most characters of a caller's member id a record keeps (#2303
/// review L3): the id is the caller's own text, bounded by the board only
/// once the member is admitted, and a refused call can name any.
pub const ACTOR_REF_CHARS: usize = 128;

/// The telemetry level of a call (#2270 round-3 review N3): a method that
/// only reads the board records at DEBUG; anything else (a mutation, or a
/// name that is no method) at INFO. A contended refusal or a slow lock
/// records at WARN.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Level {
    Read,
    Mutation,
}

/// One finished call, as its records describe it.
pub(super) struct Finished<'a> {
    pub op: &'static str,
    pub level: Level,
    pub role: BoardRole,
    pub member: &'a str,
    /// The decision taken, or the refusal's kind.
    pub outcome: Result<&'static str, RefusalKind>,
    pub elapsed: Duration,
}

/// What a served call decided, and what it acted on, for its records.
/// Every arm of the dispatcher fills every field, so none is left to a
/// default: a field that does not apply to the method is `None`.
pub(super) struct Served {
    pub value: Value,
    pub decision: &'static str,
    /// The task the op acted on, when it acted on one.
    pub task_id: Option<i64>,
    /// The message the op acted on, when it acted on one.
    pub message_id: Option<i64>,
    /// Whether the op moved the caller's message cursor; `None` for an op
    /// that has no cursor to move.
    pub cursor_moved: Option<bool>,
}

/// `member` redacted, then cut to its first [`ACTOR_REF_CHARS`]
/// characters (a whole character each).
pub(super) fn actor_ref(member: &str) -> Redacted {
    let redacted = Redacted::from(member);
    match redacted.char_indices().nth(ACTOR_REF_CHARS) {
        Some((end, _)) => Redacted::from(&redacted[..end]),
        None => redacted,
    }
}

/// The most member ids an [`ActorRefs`] keeps (#2303 round-3 review M1):
/// a run's members are bounded by the board, and a refused call's id is
/// redacted anew rather than kept.
#[cfg(test)] // Red: wired in the fix commit.
pub const ACTOR_REF_CACHE: usize = 64;

/// Each member's [`actor_ref`], redacted once per member rather than once
/// per call (#2303 round-3 review M1), for at most [`ACTOR_REF_CACHE`]
/// members.
#[cfg(test)] // Red: wired in the fix commit.
#[derive(Debug, Default)]
pub struct ActorRefs {
    known: Mutex<HashMap<String, Redacted>>,
}

#[cfg(test)] // Red: wired in the fix commit.
impl ActorRefs {
    /// `member`'s [`actor_ref`].
    pub(super) fn of(&self, member: &str) -> Redacted {
        // Red stub: every id is kept.
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        known
            .entry(member.to_owned())
            .or_insert_with(|| actor_ref(member))
            .clone()
    }

    /// How many members' refs are kept.
    fn len(&self) -> usize {
        self.known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// The call's `tracing` record, with its waits when they were measured.
pub(super) fn trace(call: &Finished<'_>, measure: Option<&CallMeasure>) {
    let (op, duration_us) = (call.op, micros(call.elapsed));
    // Redacted only when the record is written: a field's value is
    // evaluated only for an enabled callsite.
    let member = || actor_ref(call.member);
    let (outcome, decision, kind) = match call.outcome {
        Ok(decision) => ("ok", decision, "none"),
        Err(kind) => ("refused", "none", kind.as_str()),
    };
    let lock_wait_us = measure.map(|measure| micros(measure.lock_wait));
    let busy_wait_us = measure.map(|measure| micros(measure.busy_wait));
    let busy = measure.map(|measure| measure.busy);
    let slow = measure.is_some_and(|measure| measure.busy_wait > SLOW_LOCK);
    match (
        call.outcome == Err(RefusalKind::Contended) || slow,
        call.level,
    ) {
        (true, _) => tracing::warn!(
            target: TELEMETRY_TARGET, op, member = member().as_str(), outcome, decision, kind, duration_us,
            ?lock_wait_us, ?busy_wait_us, ?busy, "swarm board call"
        ),
        (false, Level::Read) => tracing::debug!(
            target: TELEMETRY_TARGET, op, member = member().as_str(), outcome, decision, kind, duration_us,
            ?lock_wait_us, ?busy_wait_us, ?busy, "swarm board call"
        ),
        (false, Level::Mutation) => tracing::info!(
            target: TELEMETRY_TARGET, op, member = member().as_str(), outcome, decision, kind, duration_us,
            ?lock_wait_us, ?busy_wait_us, ?busy, "swarm board call"
        ),
    }
}

/// The call's `swarm_op` record. `served` is what it answered and acted
/// on (`None` for a refusal), the answer sized as the compact JSON it
/// renders to; `measure` is `None` when the call began no transaction, and
/// its waits are then `null`.
pub(super) fn observation(
    call: &Finished<'_>,
    served: Option<&Served>,
    measure: Option<&CallMeasure>,
) -> BoardOpObservation {
    debug_assert!(
        measure.is_none_or(|measure| measure.lock_wait <= call.elapsed),
        "a call's lock wait is part of its duration"
    );
    BoardOpObservation {
        op: call.op.to_owned(),
        actor_ref: actor_ref(call.member),
        role: call.role,
        run_id: measure
            .and_then(|measure| measure.run_id.as_deref())
            .map(Redacted::from),
        task_id: served.and_then(|served| served.task_id),
        message_id: served.and_then(|served| served.message_id),
        outcome: match call.outcome {
            Ok(_) => BoardOpOutcome::Ok,
            Err(kind) => BoardOpOutcome::Refused { kind },
        },
        duration_us: micros(call.elapsed),
        lock_wait_us: measure.map(|measure| micros(measure.lock_wait)),
        busy_wait_us: measure.map(|measure| micros(measure.busy_wait)),
        busy: measure.map(|measure| measure.busy),
        cursor_moved: served.and_then(|served| served.cursor_moved),
        result_bytes: served.map_or(0, |served| rendered_bytes(&served.value)),
    }
}

/// The bytes of `value`'s compact JSON (`serde_json`'s, not Python's
/// `json.dumps` with its spaced separators), counted without building it.
fn rendered_bytes(value: &Value) -> u64 {
    struct Count(u64);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    // Writing a `Value` to a counter cannot fail.
    let _written = serde_json::to_writer(&mut count, value);
    count.0
}

#[cfg(test)]
#[path = "swarm_board_telemetry_tests.rs"]
mod tests;
