//! Where a [`super::SwarmBoard`]'s calls are recorded (#2313): the
//! session's event log, followed across a session switch, and each board
//! file's run folded for its `swarm_run_summary`.
//!
//! [`SessionLog`] is the one log every handle the board builds records in.
//! It forwards each record to the session's event log it is bound to, and
//! is rebound, not rebuilt, when the session switches, so the records
//! follow the session. Bound only once the session's log is open, it holds
//! what it is given before then (the admission's own calls, measured only
//! when the event log was decided on before admission, owner decision T1),
//! at most [`PENDING_RECORDS`], and writes them first when it is bound.
//!
//! [`RunFold`] folds each record of one board file's run
//! ([`RunSummaryFold`]) on its way to the session log, and writes the run's
//! summary once, when the coordinator's harness settles the run: this
//! process's counts, and the run's own totals read from the board then
//! ([`run_totals`], #2313 review M1).
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use crate::application::swarm::ports::BoardOpLog;
use crate::domain::swarm::telemetry::board_run_id;
use serde_json::Value;

use crate::domain::swarm::{
    BoardOpObservation, MemberUsage, MessageTotals, RunSummaryFold, RunTotals, SwarmRunSummary,
    TaskStates,
};
use crate::infrastructure::tools::swarm_board_dispatch::TELEMETRY_TARGET;
use crate::infrastructure::tools::swarm_board_telemetry::actor_ref;

/// The most records a session log holds before it is bound: the
/// admission's few calls, with room to spare. Past it a record is dropped
/// and counted.
pub(super) const PENDING_RECORDS: usize = 64;

/// A record held until the session's log is bound.
enum Held {
    Op(BoardOpObservation),
    Summary(SwarmRunSummary),
}

enum Binding {
    Pending { held: Vec<Held>, dropped: u64 },
    Bound(Arc<dyn BoardOpLog>),
}

/// The session's event log, rebound on a session switch.
pub(super) struct SessionLog {
    binding: Mutex<Binding>,
}

impl SessionLog {
    /// A log not yet bound to a session's event log: it holds its records.
    pub(super) fn pending() -> Self {
        Self {
            binding: Mutex::new(Binding::Pending {
                held: Vec::new(),
                dropped: 0,
            }),
        }
    }

    /// Records in `log` from now on; the records held until now are
    /// written to it first, in order, before any later record.
    pub(super) fn bind(&self, log: Arc<dyn BoardOpLog>) {
        let mut binding = self.held();
        if let Binding::Pending { held, dropped } = &mut *binding {
            if *dropped > 0 {
                tracing::warn!(
                    target: TELEMETRY_TARGET,
                    dropped = *dropped,
                    "swarm board records made before the event log opened were dropped"
                );
            }
            for record in held.drain(..) {
                match record {
                    Held::Op(observation) => log.record(observation),
                    Held::Summary(summary) => log.summarize(summary),
                }
            }
        }
        *binding = Binding::Bound(log);
    }

    /// Writes `record` to the bound log (outside the lock), or holds it.
    fn write(&self, record: Held) {
        let log = {
            let mut binding = self.held();
            match &mut *binding {
                Binding::Bound(log) => log.clone(),
                Binding::Pending { held, dropped } => {
                    match held.len() < PENDING_RECORDS {
                        true => held.push(record),
                        false => *dropped = dropped.saturating_add(1),
                    }
                    debug_assert!(
                        held.len() <= PENDING_RECORDS,
                        "the held records are bounded"
                    );
                    return;
                }
            }
        };
        match record {
            Held::Op(observation) => log.record(observation),
            Held::Summary(summary) => log.summarize(summary),
        }
    }

    /// The binding, also after a panic elsewhere left the lock poisoned:
    /// nothing is left half-updated under it.
    fn held(&self) -> MutexGuard<'_, Binding> {
        self.binding.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl BoardOpLog for SessionLog {
    fn record(&self, observation: BoardOpObservation) {
        self.write(Held::Op(observation));
    }

    fn summarize(&self, summary: SwarmRunSummary) {
        self.write(Held::Summary(summary));
    }
}

/// The run being folded, open until its summary is written.
struct Folding {
    fold: RunSummaryFold,
    open: bool,
}

/// One board file's run, folded on the way to the session log.
pub(super) struct RunFold {
    log: Arc<dyn BoardOpLog>,
    origin: Instant,
    run: Mutex<Option<Folding>>,
}

impl RunFold {
    pub(super) fn new(log: Arc<dyn BoardOpLog>) -> Self {
        Self {
            log,
            origin: Instant::now(),
            run: Mutex::new(None),
        }
    }

    fn now_us(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    fn folding(&self) -> MutexGuard<'_, Option<Folding>> {
        self.run.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Folds `observation` when it found a run the board generated: into
    /// that run's fold, until its summary is written; a record of another
    /// run (the file now holds a new one) starts that run's fold.
    fn fold(&self, observation: &BoardOpObservation) {
        let Some(run_id) = observation.run_id.as_deref().filter(|id| board_run_id(id)) else {
            return;
        };
        let now = self.now_us();
        let mut run = self.folding();
        match run.as_mut() {
            Some(folding) if folding.fold.run_id() == run_id => {
                if folding.open {
                    folding.fold.observe(observation);
                }
            }
            Some(_) | None => {
                let started = now.saturating_sub(observation.duration_us);
                let mut fold = RunSummaryFold::new(run_id, started);
                fold.observe(observation);
                *run = Some(Folding { fold, open: true });
            }
        }
    }

    /// Whether a run is folded whose summary is not written yet.
    pub(super) fn open(&self) -> bool {
        self.folding().as_ref().is_some_and(|folding| folding.open)
    }

    /// Writes the summary of the run folded, once, with the run's totals
    /// `_run_totals` answered (`totals`: its run-wide section only when
    /// they are the folded run's): `false` when there is no run folded, or
    /// its summary was already written.
    pub(super) fn summarize_run(&self, totals: Option<&Value>) -> bool {
        let summary = {
            let mut run = self.folding();
            match run.as_mut() {
                Some(folding) if folding.open => {
                    folding.open = false;
                    let totals =
                        totals.and_then(|totals| run_totals(totals, folding.fold.run_id()));
                    folding.fold.summary(self.now_us(), totals)
                }
                Some(_) | None => return false,
            }
        };
        tracing::info!(
            target: TELEMETRY_TARGET,
            run_id = summary.run_id.as_str(),
            records = summary.records,
            busy = summary.busy,
            process_span_us = summary.process_span_us,
            run_totals = summary.run.is_some(),
            "swarm run summary"
        );
        self.log.summarize(summary);
        true
    }
}

impl BoardOpLog for RunFold {
    fn record(&self, observation: BoardOpObservation) {
        self.fold(&observation);
        self.log.record(observation);
    }

    fn summarize(&self, summary: SwarmRunSummary) {
        self.log.summarize(summary);
    }
}

/// The run-wide section from `_run_totals`'s answer, when it is of
/// `run_id` (the file may hold a newer run by now): counts, and each
/// member as its redacted `actor_ref` (#2313 review M1). A count the board
/// does not hold as a non-negative integer is `0`.
pub(super) fn run_totals(answer: &Value, run_id: &str) -> Option<RunTotals> {
    if answer.get("run_id").and_then(Value::as_str) != Some(run_id) {
        return None;
    }
    let count = |value: &Value, key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
    let tasks = answer.get("tasks")?;
    let messages = answer.get("messages")?;
    let usage = answer
        .get("usage")
        .and_then(Value::as_array)?
        .iter()
        .map(|row| MemberUsage {
            actor_ref: match row.get("member") {
                Some(Value::String(member)) => actor_ref(member),
                Some(other) => actor_ref(&other.to_string()),
                None => actor_ref("null"),
            },
            requests: count(row, "requests"),
            tokens: count(row, "observed_tokens"),
            unknown_usage_requests: count(row, "unknown_usage_requests"),
            attempts: count(row, "attempts"),
            input_tokens: count(row, "reported_input_tokens"),
            output_tokens: count(row, "reported_output_tokens"),
            cache_read_tokens: count(row, "reported_cache_read_tokens"),
            cache_write_tokens: count(row, "reported_cache_write_tokens"),
        })
        .collect();
    Some(RunTotals::new(
        TaskStates {
            total: count(tasks, "total"),
            ready: count(tasks, "ready"),
            claimed: count(tasks, "claimed"),
            blocked: count(tasks, "blocked"),
            submitted: count(tasks, "submitted"),
            completed: count(tasks, "completed"),
        },
        MessageTotals {
            sent: count(messages, "sent"),
            acked: count(messages, "acked"),
            withdrawn: count(messages, "withdrawn"),
        },
        usage,
        answer.get("created_at").and_then(Value::as_f64),
        answer.get("read_at").and_then(Value::as_f64)?,
    ))
}
