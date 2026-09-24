//! What a plain `stopped` record's box left behind (#2134), judged the
//! same way by the restore and the collector: its state directory absent
//! (with its state root present) and its container reported gone by its
//! own retained `inspect` is nothing left, and the record may be
//! forgotten. The disk is asked first, so a box that left its state is
//! never inspected. A probe counts only the time inside its own inspects
//! against a budget ([`RESIDUE_INSPECT_BUDGET_MILLIS`]), and an inspect
//! that answers slowly ([`SLOW_INSPECT_MILLIS`]) marks its inspect command
//! as hanging: the other records of that config (those sharing the command)
//! wait, while other configs' records are still judged. So a hanging
//! runtime cannot hold a session start, and a record whose inspect fails
//! fast (no argv, an unrecognised status) holds back no other. What is not inspected is
//! kept, to be judged by a later restore.
use crate::domain::environment_registry::EnvironmentRecord;

use super::super::dto::{EnvironmentLiveness, StateOnDisk};
use super::super::ports::EnvironmentProcess;

/// How long, in milliseconds, one restore or collection spends inspecting
/// stopped records (one inspect started within it may run to its bound).
pub const RESIDUE_INSPECT_BUDGET_MILLIS: u64 = 15_000;

/// An inspect this slow (the script bound is 5 s) marks its inspect command,
/// and so its config, as hanging for the rest of the probe.
pub const SLOW_INSPECT_MILLIS: u64 = 4_000;

/// Why a stopped record whose workspace names no environment directory is
/// kept.
pub const NO_ENVIRONMENT_DIR: &str =
    "its workspace names no environment directory, so what it left cannot be told";

/// Why a stopped record whose state directory is gone but whose container
/// still runs is kept.
pub const STATE_GONE_CONTAINER_RUNNING: &str = "its state directory is gone but the runtime still runs its container; kill the container by hand";

/// What a stopped record's box left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Residue {
    /// Nothing on disk or in the runtime: the record may be forgotten.
    Nothing,
    /// Its state directory is on disk: the collector's to remove.
    StateOnDisk,
    /// Not inspected this time (the inspect budget is spent): judged again
    /// by a later restore.
    Deferred,
    /// Kept, with the reason worth saying.
    Kept(String),
}

/// One restore's (or collection's) judge of stopped records' boxes.
pub(super) struct ResidueProbe<'a> {
    process: &'a dyn EnvironmentProcess,
    /// Time spent inside this probe's own inspects: what else the caller
    /// inspects (live records) is not the probe's to count.
    spent_millis: u64,
    budget_millis: u64,
    /// Inspect argvs that answered slowly: a config's records share one
    /// (the environment id travels in the environment), so its other records
    /// wait for a later restore (a circuit breaker).
    hanging: Vec<Vec<String>>,
}

impl<'a> ResidueProbe<'a> {
    pub fn new(process: &'a dyn EnvironmentProcess) -> Self {
        Self::with_budget(process, RESIDUE_INSPECT_BUDGET_MILLIS)
    }

    pub fn with_budget(process: &'a dyn EnvironmentProcess, budget_millis: u64) -> Self {
        Self {
            process,
            spent_millis: 0,
            budget_millis,
            hanging: Vec::new(),
        }
    }

    /// What `record`'s box left. Meaningful for a plain stopped record;
    /// the caller decides which records to ask about.
    pub fn residue(&mut self, record: &EnvironmentRecord) -> Residue {
        match record.environment_dir() {
            Some(dir) => match self.process.state_on_disk(&dir) {
                StateOnDisk::Absent => self.runtime_residue(record),
                StateOnDisk::Present => Residue::StateOnDisk,
                StateOnDisk::Unknown(reason) => Residue::Kept(reason),
            },
            None => Residue::Kept(NO_ENVIRONMENT_DIR.to_string()),
        }
    }

    fn runtime_residue(&mut self, record: &EnvironmentRecord) -> Residue {
        let within_budget = self.spent_millis < self.budget_millis;
        let runtime_hangs = self.hanging.contains(&record.retained_inspect_argv);
        match (within_budget, runtime_hangs) {
            (true, false) => self.inspect(record),
            (false, _) | (true, true) => Residue::Deferred,
        }
    }

    /// Run the record's own inspect, counting its time and tripping the
    /// breaker for its config when it answered slowly.
    fn inspect(&mut self, record: &EnvironmentRecord) -> Residue {
        let started = self.process.inspect_clock_millis();
        let liveness = self.process.observe(record);
        let took = self.process.inspect_clock_millis().saturating_sub(started);
        self.spent_millis = self.spent_millis.saturating_add(took);
        if took >= SLOW_INSPECT_MILLIS {
            self.hanging.push(record.retained_inspect_argv.clone());
        }
        match liveness {
            EnvironmentLiveness::Gone => Residue::Nothing,
            EnvironmentLiveness::Running => Residue::Kept(STATE_GONE_CONTAINER_RUNNING.to_string()),
            EnvironmentLiveness::Unknown(reason) => Residue::Kept(reason),
        }
    }
}

#[cfg(test)]
#[path = "box_residue_tests.rs"]
mod tests;
