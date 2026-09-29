//! `Workbench.resume()` (#2273): a member's own resume, which the board
//! always refuses (#1729).
use crate::application::swarm::dto::ControlAnswer;
use crate::domain::swarm::BoardError;

/// Why a member cannot resume: only the supervisor outside the swarm does.
pub const MEMBERS_CANNOT_RESUME: &str = "a paused run is resumed only by the supervisor outside \
     the swarm (agent_cmd swarm_control resume); members cannot resume it";

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
    /// Always [`MEMBERS_CANNOT_RESUME`].
    pub fn execute(&self, _actor: &str) -> Result<ControlAnswer, BoardError> {
        Err(BoardError::new(MEMBERS_CANNOT_RESUME))
    }
}

#[cfg(test)]
#[path = "resume_run_tests.rs"]
mod tests;
