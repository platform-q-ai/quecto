//! Controller of the `resume_session` command: maps exact restore fields onto
//! the application request. Legacy action handling remains at the wire edge.
use crate::application::sessions::dto::{ResumeRequest, ResumeSavedSessionError};
use crate::domain::resume_decision::HomeVersion;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeFields {
    pub session: String,
    pub expected_home_version: Option<String>,
    /// The request carried an `action` — present, whatever its value (even
    /// `null`): a pre-#2045 client. It is refused, never read as a restore.
    pub legacy_action: bool,
}

impl ResumeFields {
    pub fn into_request(self) -> Result<ResumeRequest, ResumeSavedSessionError> {
        if self.legacy_action {
            return Err(ResumeSavedSessionError::LegacyAction);
        }
        let expected_home_version = match self.expected_home_version {
            Some(raw) => {
                Some(HomeVersion::parse(&raw).ok_or(ResumeSavedSessionError::StaleHomeVersion)?)
            }
            None => None,
        };
        Ok(ResumeRequest {
            target: self.session,
            expected_home_version,
        })
    }
}

impl From<String> for ResumeFields {
    fn from(session: String) -> Self {
        Self {
            session,
            expected_home_version: None,
            legacy_action: false,
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
