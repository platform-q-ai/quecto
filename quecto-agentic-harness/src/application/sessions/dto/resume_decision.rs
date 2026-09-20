//! Boundary values for an exact saved-session restore and a typed home refusal.
use super::resume_saved_session::{ResumeTarget, SavedSessionResumed};
use crate::domain::resume_decision::{HomeVersion, ResumeDecisionKind};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeRequest {
    pub target: String,
    pub expected_home_version: Option<HomeVersion>,
}

impl ResumeRequest {
    pub fn restore(target: &str) -> Self {
        Self {
            target: target.to_string(),
            expected_home_version: None,
        }
    }
}

/// A target exists but its authoritative home does not admit a restore here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeDecision {
    pub target: ResumeTarget,
    pub kind: ResumeDecisionKind,
    pub home_version: HomeVersion,
    pub execution_dir: Option<PathBuf>,
    pub detail: Option<String>,
}

impl std::fmt::Display for ResumeDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "session resume unavailable: {}", self.kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOutcome {
    Resumed(SavedSessionResumed),
}

#[cfg(test)]
#[path = "resume_decision_tests.rs"]
mod tests;
