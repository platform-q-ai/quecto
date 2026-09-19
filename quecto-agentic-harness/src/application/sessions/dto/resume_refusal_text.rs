//! The stable code and the user text of every resume refusal (#2011): the
//! code is the boundary's contract, the text is for people.
use super::ResumeSavedSessionError;

impl ResumeSavedSessionError {
    /// The stable machine name of the refusal on every boundary.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Ephemeral => "ephemeral",
            Self::InvalidName => "invalid_name",
            Self::Decision(_) => "decision_required",
            Self::StaleHomeVersion => "stale_home_version",
            Self::CurrentScopeUnavailable(_) => "current_scope_unavailable",
            Self::ActionUnavailable { .. } => "action_unavailable",
            Self::ActionExecutedElsewhere(_) => "action_executed_elsewhere",
            Self::StartupScope(_) => "startup_scope",
            Self::Refused(_) => "transition_refused",
            Self::Save(_) => "save_failed",
            Self::Claim(_) => "claim_refused",
            Self::NotFound(_) => "not_found",
            Self::Load(_) => "load_failed",
        }
    }
}

impl std::fmt::Display for ResumeSavedSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ephemeral => f.write_str("cannot resume sessions in ephemeral mode"),
            Self::InvalidName => {
                f.write_str("session name must contain only alphanumeric, '-', or '_'")
            }
            Self::Decision(decision) => write!(f, "{decision}"),
            Self::StaleHomeVersion => f.write_str(
                "session home changed since it was listed; refresh the list and choose again",
            ),
            Self::CurrentScopeUnavailable(_) => f.write_str(
                "session resume unavailable: the current execution directory cannot be \
                 discovered, so no saved session can be admitted here",
            ),
            Self::ActionUnavailable { action, reason } => {
                write!(f, "{} is unavailable: {reason}", action.name())
            }
            Self::ActionExecutedElsewhere(action) => write!(
                f,
                "{} is not a restore: request it through its own command",
                action.name()
            ),
            Self::StartupScope(refusal) => write!(f, "{refusal}"),
            Self::Refused(refused) => write!(f, "{refused}"),
            Self::Save(error) => write!(f, "failed to save current session: {error}"),
            Self::Claim(error) => write!(f, "{error}"),
            Self::NotFound(name) => write!(f, "session not found: {name}"),
            Self::Load(error) => write!(f, "failed to load session: {error}"),
        }
    }
}

#[cfg(test)]
#[path = "resume_refusal_text_tests.rs"]
mod tests;
