//! Read conversation history (#1856): stable-id cursor selection and
//! chronological paging over the user-visible transcript — the live one
//! the loop publishes (idle and busy transports alike) or a persisted one
//! the store holds.
//!
//! The application owns which messages a page holds and what its cursor
//! and `has_more_before` mean (`history_paging`, shared with the sync use
//! case); the transport owns how many of them a frame can carry
//! ([`HistoryPage::keeping_newest`]).
use std::sync::Arc;

use crate::application::sessions::active_session::ActiveSessionHandle;
use crate::application::sessions::dto::{HistoryError, HistoryPage, HistoryQuery};
use crate::application::sessions::history_paging;
use crate::application::sessions::ports::SessionStore;
use crate::domain::ids::MessageId;
use crate::domain::message::Message;
use crate::domain::session_identity::SessionIdentity;

/// Application boundary conversion for exact opaque persisted keys.
pub(crate) fn exact_persisted_identity(key: impl Into<String>) -> SessionIdentity {
    SessionIdentity::from_persisted_key(key)
}

pub struct ReadHistory {
    state: ActiveSessionHandle,
    store: Arc<dyn SessionStore>,
}

impl ReadHistory {
    pub fn new(state: ActiveSessionHandle, store: Arc<dyn SessionStore>) -> Self {
        Self { state, store }
    }

    /// The window `query` selects from `conversation`, whose injected
    /// system prompt (`injected_prompt`, empty when none) is not part of the
    /// visible transcript: the shared paging rule
    /// ([`history_paging::page_of`]) over the caller's view.
    pub fn page_of(
        &self,
        conversation: &[Message],
        injected_prompt: &str,
        query: &HistoryQuery,
    ) -> Result<HistoryPage, HistoryError> {
        history_paging::page_of(conversation, injected_prompt, query)
    }

    /// The window `query` selects from the live transcript the loop last
    /// published (the busy transports' view).
    pub async fn page_live(&self, query: &HistoryQuery) -> Result<HistoryPage, HistoryError> {
        let state = self.state.read().await;
        self.page_of(state.conversation().live_messages(), "", query)
    }

    /// The window of a persisted transcript the store holds under `key`
    /// (a historical sub-agent's session, for instance). `count: None`
    /// reads the whole transcript.
    pub async fn page_persisted(
        &self,
        key: &str,
        count: Option<usize>,
        before: Option<&MessageId>,
    ) -> Result<HistoryPage, HistoryError> {
        let identity = SessionIdentity::from_persisted_key(key);
        let session = self
            .store
            .load(&identity)
            .await
            .map_err(HistoryError::Store)?
            .ok_or(HistoryError::TranscriptNotFound)?;
        let query = HistoryQuery {
            count: count.unwrap_or(session.messages.len()),
            before: before.cloned(),
        };
        self.page_of(&session.messages, "", &query)
    }
}

impl std::fmt::Debug for ReadHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadHistory").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "read_history_tests.rs"]
mod tests;
