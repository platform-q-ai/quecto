//! `Workbench.resume()` (#2273): a member's own resume, which the board
//! always refuses (#1729).
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::dto::ControlAnswer;
use crate::application::swarm::ports::BoardRepository;
use crate::domain::swarm::{BoardError, RefusalKind};

/// Every member, the coordinator included, is refused before any
/// transaction opens: the supervisor resumes through
/// `ResumeRunExternally`.
#[derive(Default)]
pub struct ResumeRun;

impl ResumeRun {
    pub fn new() -> Self {
        Self
    }

    /// # Errors
    /// Always `supervisor_only`: only the supervisor outside the swarm
    /// resumes a paused run.
    pub fn execute(&self, _actor: &str) -> Result<ControlAnswer, BoardError> {
        Err(BoardError::new(
            RefusalKind::SupervisorOnly,
            "a paused run is resumed only by the supervisor outside the swarm \
             (agent_cmd swarm_control resume); members cannot resume it",
        ))
    }
}

/// A member's resume opens no transaction, so any repository serves it
/// alike (#2303): the call's metered repository measures nothing.
impl OverRepository for ResumeRun {
    fn over(&self, _repository: Arc<dyn BoardRepository>) -> Self {
        Self
    }
}

#[cfg(test)]
#[path = "resume_run_tests.rs"]
mod tests;
