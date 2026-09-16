//! Ports for cross-folder resume side effects (#2001 D4).
//!
//! Open-original launches a fresh target-rooted runtime (not chdir).
//! Fork imports transcript only. Locate reassociates metadata.

use crate::application::sessions::dto::resume_disposition::{DispositionError, DispositionPlan};
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session_home_scope::{CanonicalExecutionLocation, SessionHomeScope};
use crate::domain::session_identity::SessionIdentity;

/// Result of launching an open-original runtime rooted at the session home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenOriginalLaunch {
    pub identity: SessionIdentity,
    pub runtime_root: CanonicalExecutionLocation,
    /// Affirmative: process CWD was NOT changed to the target (fresh root instead).
    pub launched_without_chdir: bool,
}

/// Result of forking transcript into a new opaque identity under current scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkOutcome {
    pub source: SessionIdentity,
    pub new_identity: SessionIdentity,
    pub message_count: usize,
    pub new_home: SessionHomeScope,
}

/// Result of explicit locate/reassociation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocateOutcome {
    pub identity: SessionIdentity,
    pub new_home: SessionHomeScope,
}

/// Port: launch a fresh runtime rooted at the original session home.
pub trait OpenOriginalRuntimeLauncher: Send + Sync {
    fn launch_at_root(
        &self,
        identity: &SessionIdentity,
        target_root: &CanonicalExecutionLocation,
    ) -> Result<OpenOriginalLaunch, DomainError>;
}

/// Port: create a new opaque identity and persist transcript-only copy.
pub trait ForkTranscriptPort: Send + Sync {
    fn fork_transcript(
        &self,
        source: &SessionIdentity,
        transcript: &[Message],
        new_home: &SessionHomeScope,
    ) -> Result<ForkOutcome, DomainError>;
}

/// Port: persist explicit home reassociation without rewriting opaque keys.
pub trait LocateHomePort: Send + Sync {
    fn reassociate(
        &self,
        identity: &SessionIdentity,
        new_home: &SessionHomeScope,
    ) -> Result<LocateOutcome, DomainError>;
}

/// Port: load transcript messages for fork (authority = session store).
pub trait TranscriptSource: Send + Sync {
    fn load_messages(&self, identity: &SessionIdentity) -> Result<Vec<Message>, DomainError>;
}

/// Apply a planned disposition through the ports (application orchestration helper).
pub trait DispositionExecutor: Send + Sync {
    fn apply(&self, plan: DispositionPlan) -> Result<DispositionApplyResult, DispositionError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispositionApplyResult {
    SameScope { identity: SessionIdentity },
    Opened(OpenOriginalLaunch),
    Forked(ForkOutcome),
    Located(LocateOutcome),
    Cancelled,
}

impl DispositionApplyResult {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

#[cfg(test)]
#[path = "resume_runtime_tests.rs"]
mod tests;
