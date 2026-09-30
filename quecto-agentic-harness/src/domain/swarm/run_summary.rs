//! The run summary (#2313, epic #2265): one `swarm_run_summary` record per
//! run, written once by the coordinator's harness when the run settles (it
//! ended, or was cancelled). It holds two scopes (#2313 review M1):
//!
//! - the process's own: a pure aggregate of the `swarm_op` records
//!   ([`BoardOpObservation`]) this harness wrote for the run. Each member
//!   is a process with a board of its own, so these are the coordinator's
//!   calls only; `quecto swarm report` (#2305) folds every member's records
//!   of a run offline with the same [`RunSummaryFold`];
//! - the run's ([`RunTotals`]): the board's totals for every member, read
//!   from the board file at settle.
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
use super::run_totals::RunTotals;
use super::telemetry::{BoardOpObservation, BoardOpOutcome, RefusalKind, board_run_id};
use crate::domain::redaction::Redacted;

/// The most samples of each measure an op keeps for its percentiles: a
/// uniform sample of all its records (reservoir sampling, #2313 review
/// L2), the records not held counted in [`OpSummary::unsampled`]. The max
/// is exact whatever was held.
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
    /// Records not held in the op's sample of [`SAMPLES_PER_OP`] (a
    /// uniform sample over all its records, from which the p50 and p95 are
    /// taken; the max is exact), counted; left out when there were none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unsampled: u64,
}

/// The tasks this process's answered ops of the run changed, by what
/// they did (the run's own totals are [`RunTotals::tasks`]).
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

/// The messages this process's answered ops of the run changed, by what
/// they did (the run's own totals are [`RunTotals::messages`]).
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

/// Whose calls a summary's counts are (#2313 review M1): only this
/// process's own, the board calls of the harness that wrote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryScope {
    Process,
}

/// The `swarm_run_summary` record: this process's `swarm_op` records of
/// the run, folded, and the run's own totals from the board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwarmRunSummary {
    /// The run's id, as the board generated it (32 lowercase hex digits).
    pub run_id: String,
    /// Whose calls the counts below are: [`SummaryScope::Process`], the
    /// writing harness's own. The run's totals are [`Self::run`].
    pub scope: SummaryScope,
    /// Every `swarm_op` record of the run this process folded.
    pub records: u64,
    /// The board calls those records account for (#2338): one per record,
    /// but an aggregate of the run watch's unchanged cursor polls, which
    /// accounts for its `polls`. Each op's counts are calls. `0` in a
    /// summary written before it was kept.
    #[serde(default)]
    pub calls: u64,
    /// Each op's records, by the op's name.
    pub ops: BTreeMap<String, OpSummary>,
    /// Records whose store's busy handler fired, over every op.
    pub busy: u64,
    pub tasks: TaskCounts,
    pub messages: MessageCounts,
    /// This process's own span (#2313 review L3): from the start of the
    /// first record of the run it folded to the summary. The run's wall
    /// time, from its creation, is [`RunTotals::wall_time_us`].
    pub process_span_us: u64,
    /// Each member's requests this process recorded (`_record_request`),
    /// in the order first seen.
    pub request_usage: Vec<RequestUsage>,
    /// Records of an op past [`SUMMARY_OPS`]; left out when none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unlisted_ops: u64,
    /// Requests of a member past [`REQUEST_MEMBERS`], in no
    /// `request_usage` entry (still counted under the op's own entry,
    /// `ops._record_request`); left out when none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unlisted_requests: u64,
    /// The run's own totals, every member's work, as the board holds them
    /// at settle (#2313 review M1); `None` (written `null`) when the board
    /// could not be read then.
    pub run: Option<RunTotals>,
}

fn is_zero(count: &u64) -> bool {
    *count == 0
}

/// An op's records folded so far: its counts and its measures' samples.
#[derive(Clone, Debug, Default)]
struct OpFold {
    ok: u64,
    refused: BTreeMap<RefusalKind, u64>,
    busy: u64,
    durations: Samples,
    lock_waits: Samples,
    busy_waits: Samples,
}

/// One measure over an op's records: a uniform sample of at most
/// [`SAMPLES_PER_OP`] of the values measured (reservoir sampling), how
/// many were measured, their exact max, and the records that did not
/// measure it.
#[derive(Clone, Debug, Default)]
struct Samples {
    kept: Vec<u64>,
    measured: u64,
    max: Option<u64>,
    unmeasured: u64,
}

impl Samples {
    /// Folds one record's `value` of the measure (`None`: not measured).
    /// Once the sample is full, the record's value replaces a held one
    /// with probability [`SAMPLES_PER_OP`] over the values measured so
    /// far, so every value is equally likely to be held.
    fn observe(&mut self, value: Option<u64>, draw: &mut Draw) {
        let Some(value) = value else {
            self.unmeasured = self.unmeasured.saturating_add(1);
            return;
        };
        self.measured = self.measured.saturating_add(1);
        self.max = Some(self.max.map_or(value, |max| max.max(value)));
        match self.kept.len() < SAMPLES_PER_OP {
            true => self.kept.push(value),
            false => {
                let at = draw.below(self.measured);
                if let Some(held) = usize::try_from(at)
                    .ok()
                    .and_then(|at| self.kept.get_mut(at))
                {
                    *held = value;
                }
            }
        }
        debug_assert!(
            self.kept.len() <= SAMPLES_PER_OP,
            "a measure keeps at most SAMPLES_PER_OP samples"
        );
    }

    /// The values measured but not held in the sample.
    fn unsampled(&self) -> u64 {
        let kept = u64::try_from(self.kept.len()).unwrap_or(u64::MAX);
        self.measured.saturating_sub(kept)
    }

    fn percentiles(&self) -> Percentiles {
        Percentiles {
            max: self.max,
            unmeasured: self.unmeasured,
            ..percentiles(&self.kept)
        }
    }
}

/// The fold's pseudo-random draws (splitmix64), seeded from the run's id:
/// the same records give the same summary, and nothing reads a clock or
/// the system's entropy.
#[derive(Clone, Debug)]
struct Draw(u64);

impl Draw {
    /// Seeded by the FNV-1a hash of `run_id`.
    fn for_run(run_id: &str) -> Self {
        let seed = run_id
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
            });
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        mixed ^ (mixed >> 31)
    }

    /// A draw in `0..bound` (`bound` at least 1), by Lemire's multiply.
    fn below(&mut self, bound: u64) -> u64 {
        debug_assert!(bound > 0, "a draw below nothing");
        let wide = u128::from(self.next()) * u128::from(bound);
        u64::try_from(wide >> 64).unwrap_or(0)
    }
}

/// A run's records folded one by one, for [`RunSummaryFold::summary`].
#[derive(Clone, Debug)]
pub struct RunSummaryFold {
    run_id: String,
    started_at_us: u64,
    records: u64,
    /// The board calls the records account for (#2338).
    calls: u64,
    ops: BTreeMap<String, OpFold>,
    tasks: TaskCounts,
    messages: MessageCounts,
    requests: Vec<RequestUsage>,
    unlisted_ops: u64,
    unlisted_requests: u64,
    draw: Draw,
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
            calls: 0,
            ops: BTreeMap::new(),
            tasks: TaskCounts::default(),
            messages: MessageCounts::default(),
            requests: Vec::new(),
            unlisted_ops: 0,
            unlisted_requests: 0,
            draw: Draw::for_run(run_id),
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
        // An aggregate of the watch's unchanged polls (#2338) accounts for
        // each poll it holds; any other record for its one call.
        let calls = observation.detail.polls.unwrap_or(1);
        debug_assert!(calls >= 1, "a record accounts for at least one call");
        self.records = self.records.saturating_add(1);
        self.calls = self.calls.saturating_add(calls);
        let answered = match observation.outcome {
            BoardOpOutcome::Ok => true,
            BoardOpOutcome::Refused { .. } => false,
        };
        if answered {
            debug_assert!(
                calls == 1 || observation.decision.as_deref() == Some("unchanged"),
                "only an aggregate of unchanged polls holds more than one call"
            );
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
                calls,
                &mut self.draw,
            ),
            false => self.unlisted_ops = self.unlisted_ops.saturating_add(calls),
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
    /// clock [`Self::new`] was given), with the run's totals the board
    /// held then (`run`, `None` when it could not be read).
    pub fn summary(&self, at_us: u64, run: Option<RunTotals>) -> SwarmRunSummary {
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
            self.calls,
            "every call folded is counted once"
        );
        debug_assert!(self.calls >= self.records, "a record is at least one call");
        SwarmRunSummary {
            run_id: self.run_id.clone(),
            scope: SummaryScope::Process,
            records: self.records,
            calls: self.calls,
            ops,
            busy,
            tasks: self.tasks,
            messages: self.messages,
            process_span_us: at_us.saturating_sub(self.started_at_us),
            request_usage: self.requests.clone(),
            unlisted_ops: self.unlisted_ops,
            unlisted_requests: self.unlisted_requests,
            run,
        }
    }
}

/// Folds `observation`, which accounts for `calls` board calls, into its
/// op's `fold`, drawing from `draw`: each call is counted, and the record's
/// measures are one sample (an aggregate's are its slowest poll's).
fn fold_op(fold: &mut OpFold, observation: &BoardOpObservation, calls: u64, draw: &mut Draw) {
    match observation.outcome {
        BoardOpOutcome::Ok => fold.ok = fold.ok.saturating_add(calls),
        BoardOpOutcome::Refused { kind, .. } => {
            let refused = fold.refused.entry(kind).or_default();
            *refused = refused.saturating_add(1);
        }
    }
    if observation.busy == Some(true) {
        fold.busy = fold.busy.saturating_add(1);
    }
    fold.durations.observe(Some(observation.duration_us), draw);
    fold.lock_waits.observe(observation.lock_wait_us, draw);
    fold.busy_waits.observe(observation.busy_wait_us, draw);
}

fn summarized(fold: &OpFold) -> OpSummary {
    OpSummary {
        ok: fold.ok,
        refused: fold.refused.clone(),
        duration_us: fold.durations.percentiles(),
        lock_wait_us: fold.lock_waits.percentiles(),
        busy_wait_us: fold.busy_waits.percentiles(),
        busy: fold.busy,
        unsampled: fold.durations.unsampled(),
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
        self.status.terminal() && self.coordinator == actor
    }
}

#[cfg(test)]
#[path = "run_summary_tests.rs"]
mod tests;
