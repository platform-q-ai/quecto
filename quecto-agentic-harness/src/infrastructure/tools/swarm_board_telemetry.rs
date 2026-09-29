//! The board's call records (#2270, #2303): one `tracing` record per call
//! on [`TELEMETRY_TARGET`], and, when the event log is switched on, one
//! `swarm_op` record in it through the application's `BoardOpLog` port,
//! written synchronously on the call's own thread (never through an async
//! runtime). Both carry ids, kinds, durations and sizes only, never
//! argument or board text; the caller's member id is [`Redacted`]. What
//! was not measured is `None` (`null`), never a zero.
use std::time::Duration;

use serde_json::Value;

use crate::application::swarm::ports::CallMeasure;
use crate::domain::redaction::Redacted;
use crate::domain::swarm::{BoardOpObservation, BoardOpOutcome, BoardRole, RefusalKind};

/// The `tracing` target of every board call record.
pub const TELEMETRY_TARGET: &str = "quecto::swarm_board";

/// A busy wait this long is worth a warning. It is measured only while
/// the event log is on (owner decision T1), so the warning is raised only
/// then.
const SLOW_LOCK: Duration = Duration::from_millis(250);

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

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// The call's `tracing` record, with its waits when they were measured.
pub(super) fn trace(call: &Finished<'_>, measure: Option<&CallMeasure>) {
    let (op, duration_us) = (call.op, micros(call.elapsed));
    // Redacted only when the record is written: a field's value is
    // evaluated only for an enabled callsite.
    let member = || Redacted::from(call.member);
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

/// The call's `swarm_op` record. `answer` is what it answered, sized as
/// the JSON it renders to; `measure` is `None` when the call began no
/// transaction, and its waits are then `null`.
pub(super) fn observation(
    call: &Finished<'_>,
    answer: Option<&Value>,
    measure: Option<&CallMeasure>,
) -> BoardOpObservation {
    debug_assert!(
        measure.is_none_or(|measure| measure.lock_wait <= call.elapsed),
        "a call's lock wait is part of its duration"
    );
    BoardOpObservation {
        op: call.op.to_owned(),
        actor_ref: Redacted::from(call.member),
        role: call.role,
        run_id: measure
            .and_then(|measure| measure.run_id.as_deref())
            .map(Redacted::from),
        task_id: None,
        message_id: None,
        outcome: match call.outcome {
            Ok(_) => BoardOpOutcome::Ok,
            Err(kind) => BoardOpOutcome::Refused { kind },
        },
        duration_us: micros(call.elapsed),
        lock_wait_us: measure.map(|measure| micros(measure.lock_wait)),
        busy_wait_us: measure.map(|measure| micros(measure.busy_wait)),
        busy: measure.map(|measure| measure.busy),
        cursor_moved: false,
        result_bytes: answer.map_or(0, rendered_bytes),
    }
}

/// The bytes of `value`'s JSON, counted without building it.
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
