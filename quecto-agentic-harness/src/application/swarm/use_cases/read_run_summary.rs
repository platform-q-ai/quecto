//! `Workbench.summary(since=None)` (#2277, #1969): the read model every
//! member polls.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_read_models::summary;
use crate::application::swarm::dto::{ReadRunSummaryRequest, RunSummary};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{BoardError, RefusalKind};

/// The cursor must be `None` or an integer of at least 0 (Python's `type(x)
/// is int`: no boolean, float or text), checked before the gate. Through
/// the read-only gate, the summary of the run (`board_read_models`): a
/// cursor that is the board's, with no owner turned idle since it, answers
/// `unchanged`.
pub struct ReadRunSummary {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReadRunSummary {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// `summary cursor must be a nonnegative integer`, an authorisation
    /// refusal, an edited record, or the store's.
    pub fn execute(&self, request: ReadRunSummaryRequest) -> Result<RunSummary, BoardError> {
        let since = match &request.since {
            Value::Null => None,
            given => Some(given.as_u64().ok_or_else(|| {
                BoardError::new(
                    RefusalKind::Invalid,
                    "summary cursor must be a nonnegative integer",
                )
            })?),
        };
        // A cursor beyond i64 is no event id: never the board's cursor.
        let since = since.map(|since| i64::try_from(since).unwrap_or(-1));
        summary(&*self.repository, &*self.clock, Some(&request.actor), since)
    }
}

impl OverRepository for ReadRunSummary {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "read_run_summary_tests.rs"]
mod tests;
