//! STUB (#2273 red phase).
use crate::application::swarm::dto::ControlAnswer;
use crate::domain::swarm::BoardError;

/// Why a member cannot resume.
#[cfg_attr(not(test), allow(dead_code))]
pub const MEMBERS_CANNOT_RESUME: &str = "pending #2273";

#[derive(Default)]
pub struct ResumeRun;

impl ResumeRun {
    pub fn new() -> Self {
        Self
    }

    /// # Errors
    /// Pending #2273.
    pub fn execute(&self, _actor: &str) -> Result<ControlAnswer, BoardError> {
        Err(BoardError::new("pending #2273"))
    }
}

#[cfg(test)]
#[path = "resume_run_tests.rs"]
mod tests;
