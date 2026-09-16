//! List saved sessions (#1861): the query behind the UDS `list_sessions`
//! command and the TUI resume selector.
//!
//! The application owns the request scope ([`SessionListQuery`]) and the
//! typed outcome; the store adapter keeps the mechanics the port promises
//! (allowlisted session files, summary-only reads, tolerant skipping of
//! unreadable or malformed records, newest-first order); the interface
//! presents the summaries. Nothing here forms a path or reads a directory.
use std::sync::Arc;

use crate::application::sessions::dto::SessionListQuery;
use crate::application::sessions::ports::SessionStore;
use crate::domain::error::DomainError;
use crate::domain::session::SessionSummary;

/// The saved-session listing query over the session store port.
pub struct ListSessions {
    store: Arc<dyn SessionStore>,
}

impl ListSessions {
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        Self { store }
    }

    /// The summaries of the sessions `query` covers, newest first when the
    /// store knows modification times. Summary-only: a listed session may
    /// still fail a subsequent full load (see [`SessionStore::list`]); the
    /// store's own error surfaces unchanged.
    pub async fn execute(
        &self,
        query: &SessionListQuery,
    ) -> Result<Vec<SessionSummary>, DomainError> {
        self.store.list(query).await
    }
}

impl std::fmt::Debug for ListSessions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListSessions").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "list_sessions_tests.rs"]
mod tests;
