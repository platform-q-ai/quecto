//! The board's call records (#2270, #2303): one `tracing` record per call
//! on [`TELEMETRY_TARGET`], and, when the event log is switched on, one
//! `swarm_op` record in it through the application's `BoardOpLog` port,
//! written synchronously on the call's own thread (never through an async
//! runtime). Both carry ids, kinds, durations and sizes only, never
//! argument or board text; the caller's member id is [`Redacted`] and
//! bounded to [`ACTOR_REF_CHARS`] characters, since a call can name any
//! member id, admitted or not. What was not measured, or does not apply,
//! is `None` (`null`), never a zero or a `false`.
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::Value;

use super::swarm_board_dispatch::BindingFaults;
use crate::application::swarm::dto::CallMeasure;
use crate::domain::redaction::Redacted;
use crate::domain::swarm::telemetry::{decision_kind, run_role};
use crate::domain::swarm::validation::MEMBER_ID_MAX_BYTES;
use crate::domain::swarm::{
    BoardError, BoardOpDetail, BoardOpObservation, BoardOpOutcome, BoardRole, RefusalKind,
};

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
    /// `None` for a member-facing op: the caller's role is read from the
    /// run its measure found.
    pub role: Option<BoardRole>,
    pub member: &'a str,
    /// The decision taken, or the refusal's kind.
    pub outcome: Result<&'static str, RefusalKind>,
    pub elapsed: Duration,
    /// What a refusal raised while binding the arguments found wrong
    /// (#2341), by schema name; none for any other outcome.
    pub arguments: BindingFaults,
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
    /// What the decision found and did, as counts and kinds (#2277 review
    /// M1); [`BoardOpDetail::NONE`] for an op whose decision carries none.
    pub detail: BoardOpDetail,
    /// A refusal the op answers after its writes committed (#2277 review
    /// M2: `create`'s summary; final review L2: `_bootstrap`'s and
    /// `_join`'s after the placeholder, admission or activation): the call
    /// answers it, and its record keeps the decision and detail, marked
    /// committed. `None` for an answer.
    pub refused: Option<BoardError>,
    /// Whether the op changed the run's control state (#2390 review M2):
    /// its status, control generation or deadline. Set by the op's serving,
    /// next to the decision that changed it, so the run watch is nudged
    /// exactly then.
    pub controls_run: bool,
}

/// A served call split into what it answers and, for a refusal it
/// answered after its writes committed (#2277 review M2: `create`'s
/// summary; final review L2: `_bootstrap`'s and `_join`'s), what its
/// record keeps: the call answers the refusal, and the record the
/// decision and detail, marked committed.
pub(super) fn split_committed(
    answer: Result<Served, BoardError>,
) -> (Result<Served, BoardError>, Option<Served>) {
    match answer {
        Ok(mut served) => match served.refused.take() {
            Some(refusal) => {
                debug_assert!(
                    served.value.is_null(),
                    "a committed refusal answers nothing"
                );
                (Err(refusal), Some(served))
            }
            None => (Ok(served), None),
        },
        refused => (refused, None),
    }
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
/// a run's members are bounded by the board (at most 25), and only a
/// member's ref is ever kept.
pub const ACTOR_REF_CACHE: usize = 64;

/// Who the board found the caller to be, for [`ActorRefs::of`] (#2303
/// round-4 review L1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Caller {
    /// The board accepted the call from a member of its run: an op that
    /// checks membership (or makes the caller one) answered it, or the
    /// operation gate authorised the caller before the op refused it
    /// (#2313 review M2).
    Member,
    /// Anything else: a refusal of the gate itself (or before it), or a
    /// membership-free op, which any id can make.
    Unproven,
}

/// Each member's [`actor_ref`], redacted once per member rather than once
/// per call (#2303 round-3 review M1). Only a [`Caller::Member`] whose id
/// is within the board's [`MEMBER_ID_MAX_BYTES`] is kept (round-4 review
/// L1, L2), for at most [`ACTOR_REF_CACHE`] members; any other caller's
/// id is redacted afresh on every call and never stored, so no id a
/// caller makes up can take a member's place.
#[derive(Debug, Default)]
pub struct ActorRefs {
    known: Mutex<HashMap<String, Redacted>>,
}

impl ActorRefs {
    /// `member`'s [`actor_ref`]: the one kept for it, or redacted now,
    /// outside the lock (round-4 review N4), and kept when `caller` is a
    /// member with an id within the member-id bound, while fewer than
    /// [`ACTOR_REF_CACHE`] members are.
    pub(super) fn of(&self, member: &str, caller: Caller) -> Redacted {
        if let Some(kept) = self.held().get(member) {
            return kept.clone();
        }
        let redacted = actor_ref(member);
        let keep = caller == Caller::Member && member.len() <= MEMBER_ID_MAX_BYTES;
        if keep {
            let mut known = self.held();
            if known.len() < ACTOR_REF_CACHE {
                known
                    .entry(member.to_owned())
                    .or_insert_with(|| redacted.clone());
            }
            debug_assert!(known.len() <= ACTOR_REF_CACHE, "the kept refs are bounded");
        }
        redacted
    }

    /// The kept refs, also after a panic elsewhere left the lock poisoned:
    /// nothing is ever left half-updated under it.
    fn held(&self) -> MutexGuard<'_, HashMap<String, Redacted>> {
        self.known.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How many members' refs are kept.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.held().len()
    }
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// The call's `tracing` record, with its waits when they were measured.
/// `actor` is the caller's ref when it is already at hand (the event log
/// is on); otherwise it is redacted only if the record is written.
pub(super) fn trace(call: &Finished<'_>, measure: Option<&CallMeasure>, actor: Option<&Redacted>) {
    let (op, duration_us) = (call.op, micros(call.elapsed));
    // A field's value is evaluated only for an enabled callsite.
    let member = || actor.map_or_else(|| actor_ref(call.member), Redacted::clone);
    let (outcome, decision, kind) = match call.outcome {
        Ok(decision) => ("ok", decision, "none"),
        Err(kind) => ("refused", "none", kind.as_str()),
    };
    let lock_wait_us = measure.map(|measure| micros(measure.lock_wait));
    let busy_wait_us = measure.map(|measure| micros(measure.busy_wait));
    let busy = measure.map(|measure| measure.busy);
    let commit_us = measure.map(|measure| micros(measure.commit));
    let slow = measure.is_some_and(|measure| measure.busy_wait > SLOW_LOCK);
    match (
        call.outcome == Err(RefusalKind::Contended) || slow,
        call.level,
    ) {
        (true, _) => tracing::warn!(
            target: TELEMETRY_TARGET, op, member = member().as_str(), outcome, decision, kind, duration_us,
            ?lock_wait_us, ?busy_wait_us, ?busy, ?commit_us, "swarm board call"
        ),
        (false, Level::Read) => tracing::debug!(
            target: TELEMETRY_TARGET, op, member = member().as_str(), outcome, decision, kind, duration_us,
            ?lock_wait_us, ?busy_wait_us, ?busy, ?commit_us, "swarm board call"
        ),
        (false, Level::Mutation) => tracing::info!(
            target: TELEMETRY_TARGET, op, member = member().as_str(), outcome, decision, kind, duration_us,
            ?lock_wait_us, ?busy_wait_us, ?busy, ?commit_us, "swarm board call"
        ),
    }
}

/// The call's `swarm_op` record, by `actor` (its [`actor_ref`]), whom the
/// board accepted as a member of its run or not (`caller`, #2313: only a
/// member's role is recorded).
/// `served` is what it answered and acted on (`None` for a refusal, but
/// for one after its writes committed, whose decision and detail are
/// recorded and whose refusal is marked committed), an answer sized as
/// the compact JSON it renders to; `measure` is `None`
/// when the call began no transaction, and its waits are then `null`. The
/// run id is the board's own (a uuid it generated): the meter records
/// only such an id (`board_run_id`), never one edited from outside.
pub(super) fn observation(
    call: &Finished<'_>,
    actor: Redacted,
    caller: Caller,
    served: Option<&Served>,
    measure: Option<CallMeasure>,
) -> BoardOpObservation {
    debug_assert!(
        measure
            .as_ref()
            .is_none_or(|measure| measure.lock_wait <= call.elapsed),
        "a call's lock wait is part of its duration"
    );
    debug_assert!(
        measure
            .as_ref()
            .is_none_or(|measure| measure.lock_wait + measure.commit <= call.elapsed),
        "a call's lock wait and commits are parts of its duration"
    );
    debug_assert!(
        served.is_none_or(|served| decision_kind(served.decision)),
        "a decision is a kind, never text"
    );
    debug_assert!(
        served.is_some() || call.outcome.is_err(),
        "an answered call served its answer"
    );
    debug_assert!(
        call.outcome.is_err() || call.arguments.is_empty(),
        "argument faults are recorded for a refusal only"
    );
    let measure = measure.as_ref();
    BoardOpObservation {
        op: call.op.to_owned(),
        actor_ref: actor,
        role: call.role.or_else(|| {
            let roles = measure.and_then(|measure| measure.run_roles.as_ref())?;
            run_role(
                call.member,
                roles.coordinator.as_deref(),
                roles.integrator.as_deref(),
                caller == Caller::Member,
            )
        }),
        run_id: measure.and_then(|measure| measure.run_id.clone()),
        task_id: served.and_then(|served| served.task_id),
        message_id: served.and_then(|served| served.message_id),
        outcome: match call.outcome {
            Ok(_) => BoardOpOutcome::Ok,
            Err(kind) => BoardOpOutcome::Refused {
                kind,
                committed: served.is_some(),
            },
        },
        duration_us: micros(call.elapsed),
        lock_wait_us: measure.map(|measure| micros(measure.lock_wait)),
        busy_wait_us: measure.map(|measure| micros(measure.busy_wait)),
        busy: measure.map(|measure| measure.busy),
        commit_us: measure.map(|measure| micros(measure.commit)),
        cursor_moved: served.and_then(|served| served.cursor_moved),
        result_bytes: match (call.outcome, served) {
            (Ok(_), Some(served)) => rendered_bytes(&served.value),
            _ => 0,
        },
        decision: served.map(|served| served.decision.to_owned()),
        detail: served.map_or(BoardOpDetail::NONE, |served| served.detail.clone()),
        arguments: Box::new(call.arguments.record()),
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
