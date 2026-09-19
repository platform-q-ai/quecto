//! Controller of the `resume_session` command: maps exact restore fields onto
//! the application request. Legacy action handling remains at the wire edge.
use crate::application::sessions::dto::{ResumeRequest, ResumeSavedSessionError};
use crate::domain::resume_decision::HomeVersion;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeFields {
    pub session: String,
    pub expected_home_version: Option<String>,
}

impl ResumeFields {
    pub fn into_request(self) -> Result<ResumeRequest, ResumeSavedSessionError> {
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
