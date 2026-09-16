//! Controller of the `list_sessions` command (#1861, #1970): maps the
//! scoped wire command onto the application's list query and hands the
//! summaries to the presenter. No policy: the scope is the application's
//! DTO, the order and tolerance are the store's promises.
use std::sync::Arc;

use crate::application::sessions::dto::{
    ListSessionsRequest, ListSessionsResult, SessionListQuery, SessionListScope,
};
use crate::application::sessions::use_cases::ListSessions;
use crate::domain::error::DomainError;
use crate::domain::session::SessionSummary;
use serde::{Deserialize, Serialize};

/// Discovery scope is a wire value, mapped to the application request by the controller.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionListScopeCommand {
    #[default]
    Local,
    Global,
}

pub struct ListSessionsController {
    list_sessions: Arc<ListSessions>,
}

impl ListSessionsController {
    pub fn new(list_sessions: Arc<ListSessions>) -> Self {
        Self { list_sessions }
    }

    /// Map the explicit wire scope; discovery and admission stay in the application.
    pub async fn list(
        &self,
        scope: SessionListScopeCommand,
    ) -> Result<ListSessionsResult, DomainError> {
        let scope = match scope {
            SessionListScopeCommand::Local => SessionListScope::Local,
            SessionListScopeCommand::Global => SessionListScope::Global,
        };
        self.list_sessions
            .discover(&ListSessionsRequest {
                query: SessionListQuery::All,
                scope,
            })
            .await
    }

    /// The fieldless `list_sessions` command asks for every saved session.
    pub async fn list_all(&self) -> Result<Vec<SessionSummary>, DomainError> {
        self.list_sessions.execute(&SessionListQuery::All).await
    }
}

impl std::fmt::Debug for ListSessionsController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListSessionsController")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "controller_tests.rs"]
mod tests;
