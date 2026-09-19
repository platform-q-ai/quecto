//! The user text of every resume refusal (#2011): for people. The stable
//! machine code is `resume_refusal_code.rs`.
use super::ResumeSavedSessionError;

impl std::fmt::Display for ResumeSavedSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => f.write_str("cannot resume a session while agent is running"),
            Self::Ephemeral => f.write_str("cannot resume sessions in ephemeral mode"),
            Self::InvalidName => {
                f.write_str("session name must contain only alphanumeric, '-', or '_'")
            }
            Self::Decision(decision) => {
                let path = decision
                    .execution_dir
                    .as_deref()
                    .map(crate::domain::session_path_text::display_path)
                    .unwrap_or_else(|| "the recorded folder".to_string());
                write!(f, "This session belongs to {path}.")
            }
            Self::StaleHomeVersion => {
                f.write_str("session list out of date; list and choose again")
            }
            Self::CurrentScopeUnavailable(_) => f.write_str(
                "session resume unavailable: the current execution directory cannot be \
                 discovered, so no saved session can be admitted here",
            ),
            Self::HomeVersionRequired(action) => {
                write!(f, "{} needs expectedHomeVersion; ask again", action.name())
            }
            Self::ActionNotOffered(action) => {
                write!(f, "{} is not offered for this session", action.name())
            }
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
