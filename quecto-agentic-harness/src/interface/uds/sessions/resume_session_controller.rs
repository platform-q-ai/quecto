//! Controller of the `resume_session` command (#1863, #2011): maps the wire
//! fields — the exact key, the optional explicit action and the home version
//! the client was shown — onto the application's typed resume request. No
//! policy: which homes admit a restore, which actions a decision offers and
//! which of them this runtime can execute are the application's.
use crate::application::sessions::dto::{ResumeIntent, ResumeRequest, ResumeSavedSessionError};
use crate::domain::resume_decision::{HomeVersion, ResumeAction};

/// The wire fields of a `resume_session` request, as the client sent them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeFields {
    pub session: String,
    pub action: Option<ResumeAction>,
    pub expected_home_version: Option<String>,
}

impl ResumeFields {
    /// The typed request. An absent action is a restore. A version token
    /// this harness never issued cannot be the current one: it is refused as
    /// stale, never ignored.
    pub fn into_request(self) -> Result<ResumeRequest, ResumeSavedSessionError> {
        let expected_home_version = match self.expected_home_version {
            Some(raw) => {
                Some(HomeVersion::parse(&raw).ok_or(ResumeSavedSessionError::StaleHomeVersion)?)
            }
            None => None,
        };
        Ok(ResumeRequest {
            target: self.session,
            intent: self.action.map_or(ResumeIntent::Restore, ResumeIntent::Act),
            expected_home_version,
        })
    }
}

/// The exact-key restore: `/resume <key>` with nothing else.
impl From<String> for ResumeFields {
    fn from(session: String) -> Self {
        Self {
            session,
            action: None,
            expected_home_version: None,
        }
    }
}

impl From<&str> for ResumeFields {
    fn from(session: &str) -> Self {
        Self::from(session.to_string())
    }
}

#[cfg(test)]
#[path = "resume_session_controller_tests.rs"]
mod tests;
