//! Recover full message or tool-call content (#1858): resolve a stable
//! message ref to its fullest retained copy — the ledger's full copy before
//! a possibly collapsed live entry, the retention store when only a
//! collapsed stub remains, the stub itself when the store has nothing —
//! and select the content range asked for.
//!
//! One owner for the idle dispatch loop and the busy reader task: both
//! resolve through the active session's read model; the idle loop may add
//! its own live conversation as a fallback for a ref the read model has
//! not been published yet.
use crate::application::sessions::active_session::{ActiveSessionHandle, RecoveryResolution};
use crate::application::sessions::dto::{
    ContentSelector, RecoveredContent, RecoveryError, RecoveryRequest, Utf8Range,
};
use crate::domain::conversation_view::position_by_id;
use crate::domain::ids::MessageId;
use crate::domain::message::Message;

pub struct RecoverMessage {
    state: ActiveSessionHandle,
}

impl RecoverMessage {
    pub fn new(state: ActiveSessionHandle) -> Self {
        Self { state }
    }

    /// Resolve a ref without holding the lock across retention I/O. If a
    /// lifecycle operation changes the history during that I/O, the stale
    /// result is discarded and the lookup retried against the new state;
    /// hits and fallback stubs alike are validated, so neither is ever
    /// returned from an old session.
    ///
    /// Lifecycle operations are serialised on the dispatch loop, so at most
    /// a handful of replacements can race one lookup; the retry cap only
    /// backstops pathological churn. On exhausting it the lookup resolves
    /// once more against the CURRENT state and returns its live view
    /// WITHOUT another retention read — never a stale result.
    pub async fn resolve(&self, message_id: &MessageId) -> Option<Message> {
        const MAX_RECALL_RETRIES: usize = 8;
        for _ in 0..MAX_RECALL_RETRIES {
            let resolution = {
                self.state
                    .read()
                    .await
                    .resolve_for_recovery(message_id.as_str())
            };
            let resolved = resolution.into_message().await;
            let Some(recall) = &resolved.recalled else {
                return resolved.message;
            };
            if self.state.read().await.recall_is_current(recall) {
                return resolved.message;
            }
        }
        match self
            .state
            .read()
            .await
            .resolve_for_recovery(message_id.as_str())
        {
            RecoveryResolution::Found(message) => Some(message),
            RecoveryResolution::Recall { stub, .. } => Some(stub),
            RecoveryResolution::NotFound => None,
        }
    }

    /// The content `selector` names in `message`.
    pub fn select(
        message: Message,
        selector: &ContentSelector,
    ) -> Result<RecoveredContent, RecoveryError> {
        match selector {
            ContentSelector::Message {
                offset,
                thinking_offset,
                limit,
            } => {
                let range = Utf8Range::requested(&message.content, *offset, *limit);
                Ok(RecoveredContent::Message {
                    thinking_offset: thinking_offset.unwrap_or_else(|| offset.unwrap_or(0)),
                    ranged: offset.is_some() || thinking_offset.is_some() || limit.is_some(),
                    range,
                    message: Box::new(message),
                })
            }
            ContentSelector::ToolCallArguments {
                tool_call_id,
                offset,
                limit,
            } => {
                let message_id = MessageId::from(message.id().to_string());
                let tool_call = message
                    .tool_calls
                    .iter()
                    .find(|call| call.id == tool_call_id.as_str())
                    .cloned()
                    .ok_or_else(|| RecoveryError::ToolCallNotFound {
                        message_id: message_id.clone(),
                        tool_call_id: tool_call_id.clone(),
                    })?;
                let range = Utf8Range::requested(&tool_call.arguments, *offset, *limit);
                Ok(RecoveredContent::ToolCallArguments {
                    message_id,
                    tool_call,
                    range,
                })
            }
        }
    }

    /// Resolve `request.message_id` (see [`Self::resolve`]), consulting
    /// `fallback` — the loop's own live conversation, empty for the busy
    /// transports — when the read model holds no such ref, then select the
    /// requested content.
    pub async fn execute(
        &self,
        request: &RecoveryRequest,
        fallback: &[Message],
    ) -> Result<RecoveredContent, RecoveryError> {
        let message = match self.resolve(&request.message_id).await {
            Some(message) => message,
            None => position_by_id(fallback, &request.message_id)
                .map(|index| fallback[index].clone())
                .ok_or_else(|| RecoveryError::MessageNotFound(request.message_id.clone()))?,
        };
        Self::select(message, &request.selector)
    }
}

impl std::fmt::Debug for RecoverMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoverMessage").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "recover_message_tests.rs"]
pub(crate) mod tests;
