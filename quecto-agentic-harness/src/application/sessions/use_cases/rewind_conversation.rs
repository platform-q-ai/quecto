//! Rewind a conversation (#1865, D6 #1975): the `rewind_to` transaction of
//! a loop, run after the interface admitted the request (the agent is
//! idle).
//!
//! The target is resolved first — the stable id against the full
//! conversation, the legacy index only while the conversation fits in one
//! page — and refused without any change when it names nothing, is
//! ambiguous, is out of range or is not a user message. An admitted
//! rewind then owns the exact, intentionally non-atomic sequence: the
//! selected user message and everything after it leave the live
//! conversation and the surviving messages lose their retention residue
//! (collapsed stubs are reduced to their annotation, spill ids dropped —
//! the namespace is about to be wiped, so no dangling recall may
//! survive); the ledger is reset to the truncated conversation so a
//! rewound-away ref stops resolving while survivors still do (#1060
//! review r4); the persisted watermark falls to zero; the runtime's turn
//! accounting and pending queue are reset and told the new visible count;
//! the retention namespace is cleared best-effort (a failure is logged and
//! the rewind goes on); then the session is saved. A save failure is
//! returned as the persistence error and leaves the rewound state in
//! place: no rollback, staging or cleanup-status result exists.
use std::sync::Arc;

use super::SaveSession;
use super::clear_conversation::{clear_retention_best_effort, retention_of, visible_message_count};
use crate::application::context_pruning::messages::message_stub_without_recall;
use crate::application::sessions::active_session::{ActiveSessionHandle, ActiveSessionState};
use crate::application::sessions::conversation_ledger::LedgerAdvance;
use crate::application::sessions::dto::{
    RewindConversationError, RewindRequest, RewoundConversation, SaveTrigger,
};
use crate::application::sessions::ports::session_runtime::TurnAccountingReset;
use crate::domain::conversation_edit::{resolve_rewind_target, truncate_at_user_message};
use crate::domain::message::{Message, Role};

pub struct RewindConversation {
    state: ActiveSessionHandle,
    save: Arc<SaveSession>,
}

impl RewindConversation {
    pub fn new(state: ActiveSessionHandle, save: Arc<SaveSession>) -> Self {
        Self { state, save }
    }

    /// Rewind `messages` (the loop's live conversation) to the user
    /// message `request` names and everything the loop retains about it,
    /// then save.
    pub async fn execute(
        &self,
        messages: &mut Vec<Message>,
        accounting: &mut dyn TurnAccountingReset,
        request: &RewindRequest,
    ) -> Result<RewoundConversation, RewindConversationError> {
        let message_index =
            resolve_rewind_target(messages, &request.target, request.legacy_index_window)?;
        if !rewind_to_message_index(messages, message_index) {
            return Err(RewindConversationError::InvalidTarget);
        }
        let (ledger, visible, retention) = {
            let mut state = self.state.write().await;
            let visible = visible_message_count(messages, state.injected_system_prompt());
            state.set_persisted_watermark(0);
            let ledger = reset_ledger_to(&mut state, messages);
            (ledger, visible, retention_of(&state))
        };
        accounting.history_replaced(visible);
        clear_retention_best_effort(retention, "rewind_to").await;
        match self.save.save(messages, SaveTrigger::Routine).await {
            Ok(_) => Ok(RewoundConversation {
                message_index,
                ledger,
            }),
            Err(error) => Err(RewindConversationError::Save { ledger, error }),
        }
    }
}

/// Rewind the conversation to the user message at `message_index`: it and
/// everything after it are removed and the survivors lose their retention
/// residue. `false`, with nothing changed, when the index is out of range
/// or names a non-user message.
pub fn rewind_to_message_index(messages: &mut Vec<Message>, message_index: usize) -> bool {
    if !truncate_at_user_message(messages, message_index) {
        return false;
    }
    remove_spill_references(messages);
    true
}

/// Strip retention residue from the surviving messages once the namespace
/// is about to be wiped. Role-aware (#1046: `is_collapsed` no longer
/// implies a tool stub): collapsed tool results are blanked, but collapsed
/// user/assistant messages must stay non-empty provider turns — their stub
/// is reduced to its annotation with the now-dangling `recall("…")` clause
/// stripped, honouring the same no-dangling-recall invariant the tool side
/// pins. The retention manifest goes with the entries it indexed.
fn remove_spill_references(messages: &mut Vec<Message>) {
    messages.retain(|message| !message.is_manifest);
    for message in messages {
        if message.is_collapsed {
            match message.role {
                Role::User | Role::Assistant => {
                    message.content = message_stub_without_recall(&message.content);
                    message.invalidate_token_cache();
                }
                _ => message.content.clear(),
            }
            message.is_collapsed = false;
        }
        message.spill_id = None;
    }
}

/// Reset the ledger to exactly `messages`: drop the whole prior ledger (so
/// refs from the truncated conversation stop resolving) and re-seed the
/// live view and ledger from the survivors, in one write, so a busy reader
/// observes neither the old refs nor a gap.
fn reset_ledger_to(state: &mut ActiveSessionState, messages: &[Message]) -> LedgerAdvance {
    let cleared = state.clear();
    let published = state.publish(messages);
    LedgerAdvance {
        epoch: state.conversation().epoch(),
        rev: state.conversation().rev(),
        changed: cleared.changed || published.changed,
    }
}

impl std::fmt::Debug for RewindConversation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RewindConversation").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "rewind_conversation_tests.rs"]
mod tests;
