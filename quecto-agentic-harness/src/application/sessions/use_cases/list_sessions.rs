//! List saved sessions (#1861): the query behind the UDS `list_sessions`
//! command and the TUI resume selector.
//!
//! The application owns the request scope ([`SessionListQuery`]) and the
//! typed outcome; the store adapter keeps the mechanics the port promises
//! (allowlisted session files, summary-only reads, tolerant skipping of
//! unreadable or malformed records, newest-first order); the interface
//! presents the summaries. Nothing here forms a path or reads a directory.
use std::sync::Arc;

use crate::application::sessions::dto::{
    ListSessionsRequest, ListSessionsResult, ListedSession, SessionListQuery, SessionListScope,
};
use crate::application::sessions::ports::SessionStore;
use crate::application::sessions::session_home::SessionHomeContext;
use crate::domain::error::DomainError;
use crate::domain::session::SessionSummary;
use crate::domain::session_home::SessionHomeScope;

/// The saved-session listing query over the session store port.
pub struct ListSessions {
    store: Arc<dyn SessionStore>,
    home: Option<SessionHomeContext>,
}

impl ListSessions {
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        Self { store, home: None }
    }

    pub fn with_home(mut self, home: SessionHomeContext) -> Self {
        self.home = Some(home);
        self
    }

    /// Discovery is summary-only; eligibility here is advisory and resume rechecks it.
    pub async fn discover(
        &self,
        request: &ListSessionsRequest,
    ) -> Result<ListSessionsResult, DomainError> {
        let summaries = self.store.list(&request.query).await?;
        let mut result = ListSessionsResult {
            sessions: Vec::new(),
            diagnostics: Vec::new(),
            rebuilt: false,
        };
        let current = match &self.home {
            Some(context) => match context.current() {
                Ok(home) => Some(home),
                Err(error) => {
                    result.diagnostics.push(error.to_string());
                    None
                }
            },
            None => {
                result
                    .diagnostics
                    .push("workspace discovery unavailable".into());
                None
            }
        };
        let mut catalogue_available = false;
        let entries = match &self.home {
            Some(context) => match context.catalogue.list() {
                Ok(snapshot) => {
                    catalogue_available = true;
                    result.diagnostics.extend(snapshot.diagnostics);
                    result.rebuilt = snapshot.rebuilt;
                    snapshot.entries
                }
                Err(error) => {
                    result.diagnostics.push(error.to_string());
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        for summary in summaries {
            let home = match entries
                .iter()
                .find(|(identity, _)| identity.runtime_key() == summary.key)
            {
                Some((_, home)) => home.clone(),
                None if catalogue_available => continue,
                None => SessionHomeScope::Unavailable("authoritative home unavailable".into()),
            };
            let local = matches!((&home, &current), (SessionHomeScope::Scoped(home), Some(current)) if home.group == current.group);
            if request.scope == SessionListScope::Global || local {
                let resume_eligible = self
                    .home
                    .as_ref()
                    .is_some_and(|context| context.eligible(&home));
                result.sessions.push(ListedSession {
                    summary,
                    home,
                    resume_eligible,
                });
            }
        }
        Ok(result)
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
