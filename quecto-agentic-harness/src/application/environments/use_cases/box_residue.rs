//! What a plain `stopped` record's box left behind (#2134), judged the
//! same way by the restore and the collector: its state directory absent
//! (with its state root present) and its container reported gone by its
//! own retained `inspect` is nothing left, and the record may be
//! forgotten. The disk is asked first, so a box that left its state is
//! never inspected. Inspects share a time budget per probe
//! ([`RESIDUE_INSPECT_BUDGET_MILLIS`]), so a hanging runtime cannot hold a
//! session start, while a record whose inspect fails fast (no argv, an
//! unrecognised status) costs next to nothing and holds back no other.
//! What is not inspected is kept, to be judged by a later restore.
use crate::domain::environment_registry::EnvironmentRecord;

use super::super::dto::{EnvironmentLiveness, StateOnDisk};
use super::super::ports::EnvironmentProcess;

/// How long, in milliseconds, one restore or collection spends inspecting
/// stopped records (one inspect started within it may run to its bound).
pub const RESIDUE_INSPECT_BUDGET_MILLIS: u64 = 15_000;

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
    began_millis: u64,
    budget_millis: u64,
}

impl<'a> ResidueProbe<'a> {
    pub fn new(process: &'a dyn EnvironmentProcess) -> Self {
        Self::with_budget(process, RESIDUE_INSPECT_BUDGET_MILLIS)
    }

    pub fn with_budget(process: &'a dyn EnvironmentProcess, budget_millis: u64) -> Self {
        Self {
            process,
            began_millis: process.inspect_clock_millis(),
            budget_millis,
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
        let spent = self
            .process
            .inspect_clock_millis()
            .saturating_sub(self.began_millis);
        if spent < self.budget_millis {
            match self.process.observe(record) {
                EnvironmentLiveness::Gone => Residue::Nothing,
                EnvironmentLiveness::Running => {
                    Residue::Kept(STATE_GONE_CONTAINER_RUNNING.to_string())
                }
                EnvironmentLiveness::Unknown(reason) => Residue::Kept(reason),
            }
        } else {
            Residue::Deferred
        }
    }
}

#[cfg(test)]
#[path = "box_residue_tests.rs"]
mod tests;
