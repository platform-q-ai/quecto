//! Controller of the `list_sessions` command (#1861, #1970): maps the
//! fieldless wire command onto the application's list query and hands the
//! summaries to the presenter. No policy: the scope is the application's
//! DTO, the order and tolerance are the store's promises.
use std::sync::Arc;

use crate::application::sessions::dto::SessionListQuery;
use crate::application::sessions::use_cases::ListSessions;
use crate::domain::error::DomainError;
use crate::domain::session::SessionSummary;

pub struct ListSessionsController {
    list_sessions: Arc<ListSessions>,
}

impl ListSessionsController {
    pub fn new(list_sessions: Arc<ListSessions>) -> Self {
        Self { list_sessions }
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
