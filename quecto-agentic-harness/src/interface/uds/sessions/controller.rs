//! Controller of the `list_sessions` command (#1861, #1970, #2009): hands
//! the application's scoped list query to the single query owner and the
//! typed result to the presenter. No policy: the scope is the
//! application's DTO, the order and tolerance are the store's promises.
use std::sync::Arc;

use crate::application::sessions::dto::{
    ListSessionsRequest, ListSessionsResult, SessionListQuery, SessionListScope,
};
use crate::application::sessions::use_cases::ListSessions;
use crate::domain::error::DomainError;

pub struct ListSessionsController {
    list_sessions: Arc<ListSessions>,
}

impl ListSessionsController {
    pub fn new(list_sessions: Arc<ListSessions>) -> Self {
        Self { list_sessions }
    }

    /// Every saved session in `scope`; discovery and admission stay in the application.
    pub async fn list(&self, scope: SessionListScope) -> Result<ListSessionsResult, DomainError> {
        self.list_sessions
            .discover(&ListSessionsRequest {
                query: SessionListQuery::All,
                scope,
            })
            .await
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
