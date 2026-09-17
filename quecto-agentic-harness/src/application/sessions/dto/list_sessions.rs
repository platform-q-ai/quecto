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

#[derive(Debug, Clone)]
pub struct ListSessionsResult {
    pub sessions: Vec<ListedSession>,
    pub diagnostics: Vec<String>,
    pub rebuilt: bool,
}
