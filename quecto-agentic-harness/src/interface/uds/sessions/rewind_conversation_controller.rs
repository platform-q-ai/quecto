//! Controller of the `rewind_to` command (#1865, D6 #1975): maps the wire
//! fields — the stable `messageId` every client since #1061 sends, and
//! the restricted legacy `messageIndex` (#1059) — onto the application's
//! typed rewind request. No policy: which target wins, when a legacy
//! index is ambiguous, what a valid boundary is and the transaction's
//! order are the domain's and the use case's; the protocol's history
//! page size is the wire module's.
use crate::application::sessions::dto::{RewindConversationError, RewindRequest};
use crate::domain::conversation_edit::RewindTarget;
use crate::domain::ids::MessageId;

/// The wire fields of a `rewind_to` request, as the client sent them
/// (either may be absent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindFields {
    pub message_id: Option<String>,
    pub message_index: Option<usize>,
}

impl RewindFields {
    /// The typed request: the stable id when given, else the legacy index
    /// — unambiguous only while the conversation fits in
    /// `legacy_index_window` messages — else the missing-target refusal.
    pub fn into_request(
        self,
        legacy_index_window: usize,
    ) -> Result<RewindRequest, RewindConversationError> {
        let target = RewindTarget::select(self.message_id.map(MessageId::from), self.message_index)
            .ok_or(RewindConversationError::MissingTarget)?;
        Ok(RewindRequest {
            target,
            legacy_index_window,
        })
    }
}

#[cfg(test)]
#[path = "rewind_conversation_controller_tests.rs"]
mod tests;
