//! Controller of the `get_message` command (#1858, #1971): maps the wire
//! fields (`messageId`, optional `toolCallId`, `offset`, `thinkingOffset`,
//! `limit`) onto the application's recovery request. The idle dispatch
//! loop and the busy reader task both come through here; only the idle
//! loop hands over its live conversation as the fallback for a ref the
//! read model has not been published yet.
use std::sync::Arc;

use crate::application::sessions::dto::{
    ContentSelector, RecoveredContent, RecoveryError, RecoveryRequest,
};
use crate::application::sessions::use_cases::RecoverMessage;
use crate::domain::ids::{MessageId, ToolCallId};
use crate::domain::message::Message;

/// The `get_message` wire fields, already parsed by the transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetMessageFields<'a> {
    pub message_id: &'a str,
    pub tool_call_id: Option<&'a str>,
    pub offset: Option<usize>,
    pub thinking_offset: Option<usize>,
    pub limit: Option<usize>,
}

impl GetMessageFields<'_> {
    fn request(&self) -> RecoveryRequest {
        let selector = match self.tool_call_id {
            Some(tool_call_id) => ContentSelector::ToolCallArguments {
                tool_call_id: ToolCallId::from(tool_call_id),
                offset: self.offset,
                limit: self.limit,
            },
            None => ContentSelector::Message {
                offset: self.offset,
                thinking_offset: self.thinking_offset,
                limit: self.limit,
            },
        };
        RecoveryRequest {
            message_id: MessageId::from(self.message_id),
            selector,
        }
    }
}

pub struct RecoverMessageController {
    recover_message: Arc<RecoverMessage>,
}

impl RecoverMessageController {
    pub fn new(recover_message: Arc<RecoverMessage>) -> Self {
        Self { recover_message }
    }

    /// Recover the content `fields` select. `fallback` is the idle loop's
    /// live conversation (empty for the busy transports).
    pub async fn recover(
        &self,
        fields: GetMessageFields<'_>,
        fallback: &[Message],
    ) -> Result<RecoveredContent, RecoveryError> {
        self.recover_message
            .execute(&fields.request(), fallback)
            .await
    }
}

impl std::fmt::Debug for RecoverMessageController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoverMessageController")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "recover_message_controller_tests.rs"]
mod tests;
