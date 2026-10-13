//! A fold-back's `board_fold_back` record (S4, #2479): the
//! [`FoldBackRecorder`] port over the board's recording. The application's
//! caller (the keeper that watches the swarm) is handed one by composition
//! only while the event log is on, and passes it to
//! [`FoldBackRun::execute_recorded`](crate::application::project_board::use_cases::FoldBackRun::execute_recorded):
//! one record per fold-back.
//!
//! A record holds kinds the code spells (an
//! [`ErrorCode`](super::answer::ErrorCode), a rule's code, a run outcome),
//! counts and the task's redacted slug; never a swarm's titles, owners or
//! evidence.
use std::sync::Arc;

use super::answer::error_code;
use super::recording::{micros, refusal_parts};
use crate::application::project_board::fold_back::FoldedBack;
use crate::application::project_board::ports::{FoldBackOutcome, FoldBackRecorder};
use crate::application::project_board::telemetry::BoardRecording;
use crate::domain::project_board::value_objects::telemetry::{
    FoldBackRecord, OpOutcome, StaticKind, TaskRef,
};

/// Fold-backs recorded in a board recording.
pub struct FoldBackCall {
    recording: Arc<BoardRecording>,
}

impl FoldBackCall {
    /// Fold-backs recorded in `recording`.
    pub fn new(recording: Arc<BoardRecording>) -> Self {
        Self { recording }
    }
}

impl FoldBackRecorder for FoldBackCall {
    /// Writes the fold-back's record: `ok` for a run folded or found
    /// ended, else refused with its answer's code and any rule's.
    fn record(&self, outcome: FoldBackOutcome<'_>) {
        let result = match outcome.result {
            Ok(FoldedBack::Folded(_) | FoldedBack::Ended(_)) => OpOutcome::Ok,
            Err(error) => OpOutcome::Refused {
                code: StaticKind::new(error_code(error).as_str()),
                rule: refusal_parts(error).0.map(StaticKind::new),
            },
        };
        let mut record = FoldBackRecord {
            task: TaskRef::of(outcome.task),
            run: u64::from(outcome.run),
            outcome: result,
            run_outcome: None,
            items: None,
            unmatched: None,
            extra: None,
            evidence: None,
            evidence_dropped: None,
            cost_tokens: None,
            attempts: None,
            lost: None,
            bytes: None,
            duration_us: 0,
        };
        outcome.facts.fill(&mut record);
        record.duration_us = micros(outcome.elapsed);
        self.recording.record_fold_back(record);
    }
}

#[cfg(test)]
#[path = "fold_back_call_tests.rs"]
mod tests;
