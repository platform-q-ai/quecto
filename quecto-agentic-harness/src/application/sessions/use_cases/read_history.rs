//! Read conversation history (#1856): stable-id cursor selection and
//! chronological paging over the user-visible transcript — the live one
//! the loop publishes (idle and busy transports alike) or a persisted one
//! the store holds.
//!
//! The application owns which messages a page holds and what its cursor
//! and `has_more_before` mean; the transport owns how many of them a frame
//! can carry ([`HistoryPage::keeping_newest`]).
use std::sync::Arc;

use crate::application::sessions::active_session::ActiveSessionHandle;
use crate::application::sessions::dto::{HistoryError, HistoryPage, HistoryQuery};
use crate::application::sessions::ports::SessionStore;
use crate::domain::conversation_view::{position_by_id, user_visible_messages};
use crate::domain::ids::MessageId;
use crate::domain::message::Message;
use crate::domain::session_identity::SessionIdentity;

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
    /// visible transcript.
    ///
    /// A cursor must name a message of the conversation, else it is refused
    /// as unknown. A cursor naming a message outside the visible transcript
    /// selects the newest window. `count: 0` is the empty page and reports
    /// no cursor. An explicit `count` above one page keeps the "last N"
    /// contract.
    pub fn page_of(
        &self,
        conversation: &[Message],
        injected_prompt: &str,
        query: &HistoryQuery,
    ) -> Result<HistoryPage, HistoryError> {
        if let Some(cursor) = &query.before
            && position_by_id(conversation, cursor).is_none()
        {
            return Err(HistoryError::UnknownCursor(cursor.clone()));
        }
        let visible = user_visible_messages(conversation, injected_prompt);
        Ok(Self::window(&visible, query))
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

    fn window(visible: &[Message], query: &HistoryQuery) -> HistoryPage {
        let end = query
            .before
            .as_ref()
            .and_then(|cursor| position_by_id(visible, cursor))
            .unwrap_or(visible.len());
        let start = end.saturating_sub(query.count);
        let has_more_before = query.count > 0 && start > 0;
        HistoryPage {
            messages: visible[start..end].to_vec(),
            before: has_more_before.then(|| MessageId::from(visible[start].id().to_string())),
            has_more_before,
        }
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
