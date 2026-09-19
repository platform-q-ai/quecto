//! The stable code of every resume refusal (#2011): the boundary's contract.
use super::ResumeSavedSessionError;

impl ResumeSavedSessionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::Ephemeral => "ephemeral",
            Self::InvalidName => "invalid_name",
            Self::Decision(decision) => decision.kind.refusal_code(),
            Self::StaleHomeVersion => "stale_home_version",
            Self::CurrentScopeUnavailable(_) => "current_scope_unavailable",
            Self::StartupScope(_) => "startup_scope",
            Self::Refused(_) => "transition_refused",
            Self::Save(_) => "save_failed",
            Self::Claim(_) => "claim_refused",
            Self::NotFound(_) => "not_found",
            Self::Load(_) => "load_failed",
        }
    }
}
