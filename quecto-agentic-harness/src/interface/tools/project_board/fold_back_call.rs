//! A fold-back's `board_fold_back` record (S4, #2479): what one fold-back
//! of a started task's run did. Its caller (the keeper that watches the
//! swarm) starts a [`FoldBackCall`] with the recording composition gave
//! it, hands [`FoldBackCall::facts`] to the use case, and finishes it with
//! the use case's result: one record per fold-back.
//!
//! Off means off: a call with no recording keeps no clock and gathers no
//! facts. A record holds kinds the code spells (an
//! [`ErrorCode`](super::answer::ErrorCode), a rule's code, a run outcome),
//! counts and the task's redacted slug; never a swarm's titles, owners or
//! evidence.
use std::sync::Arc;
use std::time::Instant;

use super::answer::error_code;
use super::recording::{micros, refusal_parts};
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::telemetry::{BoardRecording, FoldBackFacts};
use crate::application::project_board::use_cases::FoldedBack;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::{
    FoldBackRecord, OpOutcome, StaticKind, TaskRef,
};

/// One fold-back, and its facts while it is recorded.
pub struct FoldBackCall {
    open: Option<(Arc<BoardRecording>, Instant, FoldBackFacts)>,
}

impl FoldBackCall {
    /// A fold-back recorded in `recording` when there is one; its clock
    /// starts now.
    pub fn start(recording: Option<Arc<BoardRecording>>) -> Self {
        let open = recording.map(|recording| (recording, Instant::now(), FoldBackFacts::default()));
        Self { open }
    }

    /// Where the use case notes what it saw: `None` while unrecorded.
    pub fn facts(&mut self) -> Option<&mut FoldBackFacts> {
        self.open.as_mut().map(|(_, _, facts)| facts)
    }

    /// Writes the record of the fold-back of run `run` of task `task`,
    /// which came to `result`, when it is recorded.
    pub fn finish(self, task: &Slug, run: u32, result: &Result<FoldedBack, ProjectBoardError>) {
        let Some((recording, started, facts)) = self.open else {
            return;
        };
        let outcome = match result {
            Ok(FoldedBack::Folded(_) | FoldedBack::Ended(_)) => OpOutcome::Ok,
            Err(error) => OpOutcome::Refused {
                code: StaticKind::new(error_code(error).as_str()),
                rule: refusal_parts(error).0.map(StaticKind::new),
            },
        };
        let mut record = FoldBackRecord {
            task: TaskRef::of(task),
            run: u64::from(run),
            outcome,
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
        facts.fill(&mut record);
        record.duration_us = micros(started.elapsed());
        recording.record_fold_back(record);
    }
}

#[cfg(test)]
#[path = "fold_back_call_tests.rs"]
mod tests;
