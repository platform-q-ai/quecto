//! Typed local/global discovery requests and presentation-neutral results.
use super::SessionListQuery;
use crate::domain::session::SessionSummary;
use crate::domain::session_home::SessionHomeScope;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SessionListScope {
    #[default]
    Local,
    Global,
}

#[derive(Debug, Clone)]
pub struct ListSessionsRequest {
    pub query: SessionListQuery,
    pub scope: SessionListScope,
}

#[derive(Debug, Clone)]
pub struct ListedSession {
    pub summary: SessionSummary,
    pub home: SessionHomeScope,
    pub resume_eligible: bool,
}

impl ListedSession {
    /// The version a selection of this row echoes (#2011): the token of this
    /// identity's home as listed — the one the resume transaction recomputes
    /// from the authority and compares.
    pub fn home_version(&self) -> crate::domain::resume_decision::HomeVersion {
        crate::domain::resume_decision::HomeVersion::of(&self.summary.identity, &self.home)
    }
}

#[derive(Debug, Clone)]
pub struct ListSessionsResult {
    pub sessions: Vec<ListedSession>,
    pub diagnostics: Vec<String>,
    pub rebuilt: bool,
}
