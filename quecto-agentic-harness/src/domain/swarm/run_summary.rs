//! The run summary (#2313, epic #2265): one `swarm_run_summary` record per
//! run, a pure aggregate of the `swarm_op` records ([`BoardOpObservation`])
//! the harness wrote for that run. The coordinator's harness writes it once,
//! when the run settles (it ended, or was cancelled).
//!
//! Like the records it folds, it holds ids, kinds, durations and sizes only:
//! op names are the dispatcher's own, decisions are read only to count
//! tasks and messages, and a member appears only as its [`Redacted`]
//! `actor_ref`. Every table it keeps is bounded, so a long run cannot grow
//! it without limit: counts are always exact, and what a bound left out of
//! a table or a percentile is counted, never silently lost.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Snapshot;
use super::telemetry::{BoardOpObservation, BoardOpOutcome, RefusalKind, board_run_id};
use crate::domain::redaction::Redacted;

/// The most samples of each measure an op keeps for its percentiles; the
/// samples past it are counted in [`OpSummary::unsampled`].
pub const SAMPLES_PER_OP: usize = 4_096;

/// The most ops a summary keeps apart; a record of any other op is counted
/// in [`SwarmRunSummary::unlisted_ops`]. The dispatcher serves fewer.
pub const SUMMARY_OPS: usize = 128;

/// The most members whose request usage a summary keeps apart; a request
/// recorded by any other is counted in
/// [`SwarmRunSummary::unlisted_requests`]. A run holds at most 25.
pub const REQUEST_MEMBERS: usize = 64;

/// The percentiles of one measure over an op's records: nearest-rank p50
/// and p95, and the max, each `None` when no record measured it. A record
/// that did not measure it (`null` in the record) is left out and counted
/// in `unmeasured`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Percentiles {
    pub p50: Option<u64>,
    pub p95: Option<u64>,
    pub max: Option<u64>,
    pub unmeasured: u64,
}

/// One op's records in a run: how many were answered, how many were
/// refused by kind, and the percentiles of their measures.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpSummary {
    pub ok: u64,
    /// Refusals by kind; left out when there were none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub refused: BTreeMap<RefusalKind, u64>,
    pub duration_us: Percentiles,
    pub lock_wait_us: Percentiles,
    pub busy_wait_us: Percentiles,
    /// Records whose store's busy handler fired.
    pub busy: u64,
    /// Records past [`SAMPLES_PER_OP`], counted but in no percentile; left
    /// out when there were none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unsampled: u64,
}

/// The tasks the run's answered ops changed, by what they did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCounts {
    pub created: u64,
    pub claimed: u64,
    pub released: u64,
    pub blocked: u64,
    pub submitted: u64,
    /// Verified by the coordinator (`verify_task`).
    pub accepted: u64,
}

/// The messages the run's answered ops changed, by what they did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageCounts {
    pub sent: u64,
    /// Consumed by their recipient (`ack`).
    pub acked: u64,
    pub withdrawn: u64,
}

/// One member's provider requests the harness recorded on the board
/// (`_record_request`): how many were recorded, and how many refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestUsage {
    pub actor_ref: Redacted,
    pub recorded: u64,
    pub refused: u64,
}

/// The `swarm_run_summary` record: the run's `swarm_op` records, folded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwarmRunSummary {
    /// The run's id, as the board generated it (32 lowercase hex digits).
    pub run_id: String,
    /// Every `swarm_op` record of the run that was folded.
    pub records: u64,
    /// Each op's records, by the op's name.
    pub ops: BTreeMap<String, OpSummary>,
    /// Records whose store's busy handler fired, over every op.
    pub busy: u64,
    pub tasks: TaskCounts,
    pub messages: MessageCounts,
    /// From the start of the run's first record to the summary.
    pub wall_time_us: u64,
    /// Each member's recorded requests, in the order first seen.
    pub request_usage: Vec<RequestUsage>,
    /// Records of an op past [`SUMMARY_OPS`]; left out when none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unlisted_ops: u64,
    /// Requests of a member past [`REQUEST_MEMBERS`]; left out when none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unlisted_requests: u64,
}

fn is_zero(count: &u64) -> bool {
    *count == 0
}

/// An op's records folded so far: its counts and its kept samples.
#[derive(Clone, Debug, Default)]
struct OpFold {
    ok: u64,
    refused: BTreeMap<RefusalKind, u64>,
    busy: u64,
    unsampled: u64,
    durations: Vec<u64>,
    lock_waits: Samples,
    busy_waits: Samples,
}

/// One measure's kept samples, and the records that did not measure it.
#[derive(Clone, Debug, Default)]
struct Samples {
    kept: Vec<u64>,
    unmeasured: u64,
}

impl Samples {
    fn percentiles(&self) -> Percentiles {
        Percentiles {
            unmeasured: self.unmeasured,
            ..percentiles(&self.kept)
        }
    }
}

/// A run's records folded one by one, for [`RunSummaryFold::summary`].
#[derive(Clone, Debug)]
pub struct RunSummaryFold {
    run_id: String,
    started_at_us: u64,
    records: u64,
    ops: BTreeMap<String, OpFold>,
    tasks: TaskCounts,
    messages: MessageCounts,
    requests: Vec<RequestUsage>,
    unlisted_ops: u64,
    unlisted_requests: u64,
}

impl RunSummaryFold {
    /// A fold of run `run_id`'s records, whose first record started at
    /// `started_at_us` (on the caller's own monotonic clock).
    pub fn new(run_id: &str, started_at_us: u64) -> Self {
        assert!(
            board_run_id(run_id),
            "a run summary is kept only for a run id the board generated"
        );
        Self {
            run_id: run_id.to_owned(),
            started_at_us,
            records: 0,
            ops: BTreeMap::new(),
            tasks: TaskCounts::default(),
            messages: MessageCounts::default(),
            requests: Vec::new(),
            unlisted_ops: 0,
            unlisted_requests: 0,
        }
    }

    /// The run this fold is of.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Folds `observation`, a record of this fold's run.
    pub fn observe(&mut self, observation: &BoardOpObservation) {
        debug_assert_eq!(
            observation.run_id.as_deref(),
            Some(self.run_id.as_str()),
            "a fold takes its own run's records only"
        );
        self.records = self.records.saturating_add(1);
        let answered = match observation.outcome {
            BoardOpOutcome::Ok => true,
            BoardOpOutcome::Refused { .. } => false,
        };
        if answered {
            self.count_change(&observation.op, observation.decision.as_deref());
        }
        if observation.op == "_record_request" {
            self.count_request(&observation.actor_ref, answered);
        }
        let listed = self.ops.len() < SUMMARY_OPS || self.ops.contains_key(&observation.op);
        match listed {
            true => fold_op(
                self.ops.entry(observation.op.clone()).or_default(),
                observation,
            ),
            false => self.unlisted_ops = self.unlisted_ops.saturating_add(1),
        }
    }

    /// Counts the task or message change an answered `op` made, by the
    /// decision it took.
    fn count_change(&mut self, op: &str, decision: Option<&str>) {
        let (tasks, messages) = (&mut self.tasks, &mut self.messages);
        let counter = match (op, decision) {
            ("task_create", Some("created")) => &mut tasks.created,
            ("claim", Some("claimed")) => &mut tasks.claimed,
            ("release", Some("released")) => &mut tasks.released,
            ("block", Some("blocked")) => &mut tasks.blocked,
            ("submit", Some("submitted")) => &mut tasks.submitted,
            ("verify_task", Some("verified")) => &mut tasks.accepted,
            ("send", Some("sent")) => &mut messages.sent,
            ("ack", Some("consumed")) => &mut messages.acked,
            ("withdraw", Some("withdrawn")) => &mut messages.withdrawn,
            _ => return,
        };
        *counter = counter.saturating_add(1);
    }

    /// Counts a request `actor` recorded, or had refused.
    fn count_request(&mut self, actor: &Redacted, recorded: bool) {
        let index = match self
            .requests
            .iter()
            .position(|usage| usage.actor_ref == *actor)
        {
            Some(index) => index,
            None if self.requests.len() < REQUEST_MEMBERS => {
                self.requests.push(RequestUsage {
                    actor_ref: actor.clone(),
                    recorded: 0,
                    refused: 0,
                });
                self.requests.len() - 1
            }
            None => {
                self.unlisted_requests = self.unlisted_requests.saturating_add(1);
                return;
            }
        };
        let usage = &mut self.requests[index];
        match recorded {
            true => usage.recorded = usage.recorded.saturating_add(1),
            false => usage.refused = usage.refused.saturating_add(1),
        }
    }

    /// The summary of the records folded so far, taken at `at_us` (the
    /// clock [`Self::new`] was given).
    pub fn summary(&self, at_us: u64) -> SwarmRunSummary {
        // RED stub (#2313): the fold is taken, and an empty summary answered.
        let computed = self.computed(at_us);
        SwarmRunSummary {
            run_id: computed.run_id,
            records: 0,
            ops: BTreeMap::new(),
            busy: 0,
            tasks: TaskCounts::default(),
            messages: MessageCounts::default(),
            wall_time_us: 0,
            request_usage: Vec::new(),
            unlisted_ops: 0,
            unlisted_requests: 0,
        }
    }

    fn computed(&self, at_us: u64) -> SwarmRunSummary {
        let ops: BTreeMap<String, OpSummary> = self
            .ops
            .iter()
            .map(|(op, fold)| (op.clone(), summarized(fold)))
            .collect();
        let busy = ops
            .values()
            .fold(0_u64, |busy, op| busy.saturating_add(op.busy));
        debug_assert_eq!(
            ops.values()
                .map(|op| op.ok + op.refused.values().sum::<u64>())
                .sum::<u64>()
                + self.unlisted_ops,
            self.records,
            "every record folded is counted once"
        );
        SwarmRunSummary {
            run_id: self.run_id.clone(),
            records: self.records,
            ops,
            busy,
            tasks: self.tasks,
            messages: self.messages,
            wall_time_us: at_us.saturating_sub(self.started_at_us),
            request_usage: self.requests.clone(),
            unlisted_ops: self.unlisted_ops,
            unlisted_requests: self.unlisted_requests,
        }
    }
}

/// Folds `observation` into its op's `fold`.
fn fold_op(fold: &mut OpFold, observation: &BoardOpObservation) {
    match observation.outcome {
        BoardOpOutcome::Ok => fold.ok = fold.ok.saturating_add(1),
        BoardOpOutcome::Refused { kind, .. } => {
            let refused = fold.refused.entry(kind).or_default();
            *refused = refused.saturating_add(1);
        }
    }
    if observation.busy == Some(true) {
        fold.busy = fold.busy.saturating_add(1);
    }
    if fold.durations.len() >= SAMPLES_PER_OP {
        fold.unsampled = fold.unsampled.saturating_add(1);
        return;
    }
    fold.durations.push(observation.duration_us);
    for (samples, measured) in [
        (&mut fold.lock_waits, observation.lock_wait_us),
        (&mut fold.busy_waits, observation.busy_wait_us),
    ] {
        match measured {
            Some(value) => samples.kept.push(value),
            None => samples.unmeasured = samples.unmeasured.saturating_add(1),
        }
    }
    debug_assert!(
        fold.durations.len() <= SAMPLES_PER_OP,
        "an op keeps at most SAMPLES_PER_OP samples"
    );
}

fn summarized(fold: &OpFold) -> OpSummary {
    OpSummary {
        ok: fold.ok,
        refused: fold.refused.clone(),
        duration_us: percentiles(&fold.durations),
        lock_wait_us: fold.lock_waits.percentiles(),
        busy_wait_us: fold.busy_waits.percentiles(),
        busy: fold.busy,
        unsampled: fold.unsampled,
    }
}

/// The nearest-rank p50 and p95 and the max of `samples`; all `None` for
/// no samples.
fn percentiles(samples: &[u64]) -> Percentiles {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = |percent: usize| {
        let at = (percent * sorted.len()).div_ceil(100).max(1) - 1;
        sorted.get(at).copied()
    };
    Percentiles {
        p50: rank(50),
        p95: rank(95),
        max: sorted.last().copied(),
        unmeasured: 0,
    }
}

impl Snapshot {
    /// Whether `actor`'s harness writes the run's summary now (#2313): it
    /// is the run's coordinator, and the run has settled (ended, or was
    /// cancelled).
    pub fn summarized_by(&self, actor: &str) -> bool {
        // RED stub (#2313): no harness writes it yet.
        let _ = (self.status, actor);
        false
    }
}

#[cfg(test)]
#[path = "run_summary_tests.rs"]
mod tests;
