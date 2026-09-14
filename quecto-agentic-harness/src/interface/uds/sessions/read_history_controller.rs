//! Controller of the `get_messages` command and its supported
//! `get_messages_tail` alias (#1856, #1971): maps the wire fields (a
//! resolved count, an optional cursor string) onto the application's
//! history query, for the idle loop's live conversation, the busy
//! transports' published view, and a historical sub-agent's persisted
//! transcript. No policy: the window, the cursor and `hasMoreBefore` are
//! the use case's; the protocol's default page size is the wire module's.
use std::sync::Arc;

use crate::application::sessions::dto::{HistoryError, HistoryPage, HistoryQuery};
use crate::application::sessions::use_cases::ReadHistory;
use crate::domain::ids::MessageId;
use crate::domain::message::Message;

pub struct ReadHistoryController {
    read_history: Arc<ReadHistory>,
}

impl ReadHistoryController {
    pub fn new(read_history: Arc<ReadHistory>) -> Self {
        Self { read_history }
    }

    /// `get_messages` over the loop's live conversation: `count` messages
    /// ending at `before`, the stable id of the message the window ends at
    /// (the newest window when absent).
    pub fn page(
        &self,
        conversation: &[Message],
        injected_prompt: &str,
        count: usize,
        before: Option<&str>,
    ) -> Result<HistoryPage, HistoryError> {
        let query = HistoryQuery {
            count,
            before: before.map(MessageId::from),
        };
        self.read_history
            .page_of(conversation, injected_prompt, &query)
    }

    /// `get_messages_tail`: the supported alias for the newest `count`
    /// messages, translated here and nowhere else.
    pub fn tail(
        &self,
        conversation: &[Message],
        injected_prompt: &str,
        count: usize,
    ) -> Result<HistoryPage, HistoryError> {
        self.page(conversation, injected_prompt, count, None)
    }

    /// The newest `count` messages of an already published (user-visible)
    /// transcript: the busy `sync` resync body, read under the caller's
    /// consistent view.
    pub fn newest_page_of(&self, published: &[Message], count: usize) -> HistoryPage {
        self.read_history
            .page_of(published, "", &HistoryQuery::newest(count))
            .expect("a cursorless query has no unknown cursor")
    }

    /// The newest `count` messages of the published live transcript: the
    /// connect-time `get_messages` snapshot a busy harness pushes.
    pub async fn newest_live_page(&self, count: usize) -> HistoryPage {
        self.read_history
            .page_live(&HistoryQuery::newest(count))
            .await
            .expect("a cursorless query has no unknown cursor")
    }

    /// `get_messages` against a historical sub-agent's persisted transcript
    /// (`key`, the session key its roster entry recorded): `count` defaults
    /// to the whole transcript.
    pub async fn persisted_page(
        &self,
        key: &str,
        count: Option<usize>,
        before: Option<&str>,
    ) -> Result<HistoryPage, HistoryError> {
        let before = before.map(MessageId::from);
        self.read_history
            .page_persisted(key, count, before.as_ref())
            .await
    }
}

impl std::fmt::Debug for ReadHistoryController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadHistoryController")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "read_history_controller_tests.rs"]
mod tests;
