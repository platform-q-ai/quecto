//! Read conversation history (#1856): the paging query and its typed page.
//!
//! A page is a chronological window of the user-visible transcript ending
//! at a stable-id cursor (or at the newest message), with the cursor a
//! client continues from and whether older history remains; the transport
//! encodes and budgets its domain messages.
use crate::domain::error::DomainError;
use crate::domain::ids::MessageId;
use crate::domain::message::Message;

/// One history window: at most `count` messages ending just before the
/// message `before` names (the newest page when `None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryQuery {
    pub count: usize,
    pub before: Option<MessageId>,
}

impl HistoryQuery {
    /// The newest `count` messages.
    pub fn newest(count: usize) -> Self {
        Self {
            count,
            before: None,
        }
    }
}

/// A chronological history window. `before` names the oldest INCLUDED
/// message and is present exactly when `has_more_before`: a client pages
/// backward by asking for the window ending at it. An empty window (count
/// 0) reports no cursor.
#[derive(Debug, Clone)]
pub struct HistoryPage {
    pub messages: Vec<Message>,
    /// The transcript's report (#2226), wherever it lies, so a reader
    /// finds it without paging back.
    pub report: Option<crate::domain::turn_origin::ReportRef>,
    pub before: Option<MessageId>,
    pub has_more_before: bool,
}

impl HistoryPage {
    /// The page keeping only its newest `keep` messages (a transport budget
    /// cannot carry the window): the dropped ones are still history, so the
    /// cursor moves to the oldest kept message and older history is
    /// reported. A non-empty page always keeps its newest message.
    pub fn keeping_newest(mut self, keep: usize) -> Self {
        if keep >= self.messages.len() {
            return self;
        }
        let drop = self.messages.len() - keep.max(1);
        self.messages.drain(..drop);
        self.has_more_before = true;
        self.before = self
            .messages
            .first()
            .map(|oldest| MessageId::from(oldest.id().to_string()));
        self
    }
}

/// Why a history read produced no page.
#[derive(Debug)]
pub enum HistoryError {
    /// The cursor names no message of the conversation: stale (rewound or
    /// cleared away) or never issued. Refused, never silently restarted at
    /// the newest page, which a client would duplicate as "older" history.
    UnknownCursor(MessageId),
    /// A persisted transcript read named a session the store does not hold.
    TranscriptNotFound,
    /// The store failed to load a persisted transcript.
    Store(DomainError),
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownCursor(cursor) => write!(f, "history cursor not found: {cursor}"),
            Self::TranscriptNotFound => f.write_str("no persisted transcript"),
            Self::Store(err) => write!(f, "{err}"),
        }
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
