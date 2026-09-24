//! What a plain `stopped` record's box left behind (#2134), judged the
//! same way by the restore and the collector: its state directory absent
//! (with its state root present) and its container reported gone by its
//! own retained `inspect` is nothing left, and the record may be
//! forgotten. The disk is asked first, so a box that left its state is
//! never inspected; each probe inspects at most [`MAX_RESIDUE_INSPECTS`]
//! records and stops at the first the runtime cannot answer for, so a
//! hanging runtime cannot hold a session start. What is not inspected is
//! kept, to be judged by a later restore.
use crate::domain::environment_registry::EnvironmentRecord;

use super::super::dto::{EnvironmentLiveness, StateOnDisk};
use super::super::ports::EnvironmentProcess;

/// The most inspects one restore or collection runs for stopped records.
pub const MAX_RESIDUE_INSPECTS: usize = 20;

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
    /// Not inspected this time (the budget is spent, or the runtime has
    /// stopped answering): judged again by a later restore.
    Deferred,
    /// Kept, with the reason worth saying.
    Kept(String),
}

/// One restore's (or collection's) judge of stopped records' boxes.
pub(super) struct ResidueProbe<'a> {
    process: &'a dyn EnvironmentProcess,
    inspects_left: usize,
    runtime_answers: bool,
}

impl<'a> ResidueProbe<'a> {
    pub fn new(process: &'a dyn EnvironmentProcess) -> Self {
        Self {
            process,
            inspects_left: MAX_RESIDUE_INSPECTS,
            runtime_answers: true,
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
        match (self.runtime_answers, self.inspects_left) {
            (true, left) if left > 0 => {
                self.inspects_left = left - 1;
                match self.process.observe(record) {
                    EnvironmentLiveness::Gone => Residue::Nothing,
                    EnvironmentLiveness::Running => {
                        Residue::Kept(STATE_GONE_CONTAINER_RUNNING.to_string())
                    }
                    EnvironmentLiveness::Unknown(reason) => {
                        self.runtime_answers = false;
                        Residue::Kept(reason)
                    }
                }
            }
            _ => Residue::Deferred,
        }
    }
}

#[cfg(test)]
#[path = "box_residue_tests.rs"]
mod tests;
