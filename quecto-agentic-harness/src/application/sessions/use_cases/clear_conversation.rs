//! Clear conversation history (#1864, D6 #1975): the `clear_history`
//! transaction of a loop, run after the interface admitted the request
//! (the agent is idle).
//!
//! The transaction owns the exact, intentionally non-atomic sequence: the
//! live conversation is cleared down to its injected prompt; the ledger
//! drops every retained message and opens a new epoch so stale refs stop
//! resolving; the persisted watermark falls to zero so the next save
//! rewrites the file whole; the runtime's turn accounting and pending
//! queue are reset and told the new visible count; the retention
//! namespace of the current identity is cleared best-effort (#412: stale
//! context must not be re-injected — a failure is logged and the clear
//! goes on); then the session is saved. A save failure is returned as the
//! persistence error and leaves everything already cleared in place: no
//! rollback, staging or cleanup-status result exists.
use std::sync::Arc;

use super::SaveSession;
use crate::application::sessions::active_session::ActiveSessionHandle;
use crate::application::sessions::dto::{ClearConversationError, ClearedConversation, SaveTrigger};
use crate::application::sessions::ports::ContextSpillStore;
use crate::application::sessions::ports::session_runtime::TurnAccountingReset;
use crate::domain::conversation_edit::clear_conversation;
use crate::domain::conversation_view::is_injected_system_prompt;
use crate::domain::message::Message;
use crate::domain::session_identity::SessionIdentity;

pub struct ClearConversation {
    state: ActiveSessionHandle,
    save: Arc<SaveSession>,
}

impl ClearConversation {
    pub fn new(state: ActiveSessionHandle, save: Arc<SaveSession>) -> Self {
        Self { state, save }
    }

    /// Clear `messages` (the loop's live conversation) and everything the
    /// loop retains about it, then save.
    pub async fn execute(
        &self,
        messages: &mut Vec<Message>,
        accounting: &mut dyn TurnAccountingReset,
    ) -> Result<ClearedConversation, ClearConversationError> {
        clear_conversation(messages);
        let (ledger, visible, retention) = {
            let mut state = self.state.write().await;
            let visible = visible_message_count(messages, state.injected_system_prompt());
            state.set_persisted_watermark(0);
            let ledger = state.clear();
            (ledger, visible, retention_of(&state))
        };
        accounting.history_replaced(visible);
        clear_retention_best_effort(retention, "clear_history").await;
        match self.save.save(messages, SaveTrigger::Routine).await {
            Ok(_) => Ok(ClearedConversation { ledger }),
            Err(error) => Err(ClearConversationError::Save { ledger, error }),
        }
    }
}

/// The retention namespace of the active session: its store, if the loop
/// has one, paired with the identity it is keyed by.
pub(super) type Retention = Option<(Arc<dyn ContextSpillStore>, SessionIdentity)>;

pub(super) fn retention_of(
    state: &crate::application::sessions::active_session::ActiveSessionState,
) -> Retention {
    state
        .conversation()
        .spill_store()
        .cloned()
        .map(|store| (store, state.identity().clone()))
}

/// How many messages the transcript shows once the injected prompt is
/// hidden: what the runtime's execution view reports as the count.
pub(super) fn visible_message_count(messages: &[Message], injected_prompt: &str) -> usize {
    messages
        .iter()
        .filter(|m| !is_injected_system_prompt(m, injected_prompt))
        .count()
}

/// Clear the retention namespace, best-effort: the transaction has already
/// replaced the history, so a failure here is logged under `command` and
/// the save still runs.
pub(super) async fn clear_retention_best_effort(retention: Retention, command: &str) {
    if let Some((store, identity)) = retention
        && let Err(e) = store.clear(&identity).await
    {
        tracing::warn!("{command}: failed to clear spill store: {e}");
    }
}

impl std::fmt::Debug for ClearConversation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClearConversation").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "clear_conversation_tests.rs"]
mod tests;
