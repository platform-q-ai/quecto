//! List saved sessions (#1861): the query behind the UDS `list_sessions`
//! command and the TUI resume selector.
//!
//! The application owns the request scope ([`SessionListQuery`]) and the
//! typed outcome; the store adapter keeps the mechanics the port promises
//! (allowlisted session files, summary-only reads, tolerant skipping of
//! unreadable or malformed records, newest-first order); the interface
//! presents the summaries. Nothing here forms a path or reads a directory.
use std::sync::Arc;

use crate::application::sessions::dto::{ListSessionsRequest, ListSessionsResult};
use crate::application::sessions::ports::SessionStore;
use crate::application::sessions::session_home::SessionHomeContext;
use crate::domain::error::DomainError;

/// The saved-session listing query over the session store port.
pub struct ListSessions {
    store: Arc<dyn SessionStore>,
    home: Option<SessionHomeContext>,
}

#[path = "list_sessions_discover.rs"]
mod list_sessions_discover;

impl ListSessions {
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        Self { store, home: None }
    }

    pub fn with_home(mut self, home: SessionHomeContext) -> Self {
        self.home = Some(home);
        self
    }

    /// The sessions `request` covers, newest first when the store knows
    /// modification times, each with its home and advisory eligibility;
    /// resume rechecks admission under its claim. Summary-only: a listed
    /// session may still fail a subsequent full load (see
    /// [`SessionStore::list`]); the store's own error surfaces unchanged.
    pub async fn discover(
        &self,
        request: &ListSessionsRequest,
    ) -> Result<ListSessionsResult, DomainError> {
        let summaries = self.store.list(&request.query).await?;
        Ok(list_sessions_discover::discover(self.home.as_ref(), summaries, request.scope).await)
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
