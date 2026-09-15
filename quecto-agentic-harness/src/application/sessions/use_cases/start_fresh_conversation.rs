//! Start fresh conversation (#1862, D7 #1976): the `new_session`
//! transaction of a loop, run after the interface admitted the request
//! (the agent is idle). Distinct from resume (#1863, D8): no target, no
//! load, no claim.
//!
//! The transaction owns the baseline order exactly and decides every
//! step: the departing session's children are settled and the departing
//! session saved before anything is replaced; the roster is replaced;
//! the live conversation is cleared down to its injected prompt, the
//! persisted watermark falls to zero, and the runtime's turn accounting
//! and pending queue are reset; a fresh identity is obtained from the
//! generator — never claimed — and the store's ownership of the old
//! identity is released when the two differ; the tracker, the agent and
//! its tools adopt the new key; the ledger, identity and retention
//! namespace are replaced in one write; the session-scoped effort and
//! workflow are reset; and the fresh retention namespace is cleared
//! best-effort (a failure is logged and the switch is still reported).
//! Every refusal precedes the key replacement and keeps the current
//! session whole; children the fleet already settled stay settled. No
//! rollback, staging or cleanup-status result exists.
use std::sync::Arc;

use super::clear_conversation::{clear_retention_best_effort, retention_of, visible_message_count};
use super::{DepartingChildren, SaveSession};
use crate::application::sessions::active_session::ActiveSessionHandle;
use crate::application::sessions::dto::{
    FreshConversationStarted, SaveTrigger, SessionTransition, StartFreshConversationError,
};
use crate::application::sessions::ports::{
    FleetSettlement, FreshSessionIdentityGenerator, SessionStore, SessionSwitchRuntime,
};
use crate::domain::conversation_edit::clear_conversation;
use crate::domain::message::Message;

pub struct StartFreshConversation {
    state: ActiveSessionHandle,
    save: Arc<SaveSession>,
    store: Arc<dyn SessionStore>,
    identities: Arc<dyn FreshSessionIdentityGenerator>,
    children: Arc<DepartingChildren>,
}

impl StartFreshConversation {
    pub fn new(
        state: ActiveSessionHandle,
        save: Arc<SaveSession>,
        store: Arc<dyn SessionStore>,
        identities: Arc<dyn FreshSessionIdentityGenerator>,
        children: Arc<DepartingChildren>,
    ) -> Self {
        Self {
            state,
            save,
            store,
            identities,
            children,
        }
    }

    /// Leave the current session for a fresh one. `messages` is the loop's
    /// live conversation; `fleet` the loop's fleet teardown, if it has
    /// one; `runtime` the loop runtime the switch moves.
    pub async fn execute(
        &self,
        messages: &mut Vec<Message>,
        fleet: Option<&dyn FleetSettlement>,
        runtime: &mut dyn SessionSwitchRuntime,
    ) -> Result<FreshConversationStarted, StartFreshConversationError> {
        let transition = SessionTransition::Fresh;
        self.children
            .settle(fleet, transition)
            .await
            .map_err(StartFreshConversationError::Refused)?;
        self.save
            .save(messages, SaveTrigger::Routine)
            .await
            .map_err(StartFreshConversationError::Save)?;
        self.children
            .reset_roster(transition)
            .map_err(StartFreshConversationError::Refused)?;
        clear_conversation(messages);
        let (old_identity, visible) = {
            let mut state = self.state.write().await;
            let visible = visible_message_count(messages, state.injected_system_prompt());
            state.set_persisted_watermark(0);
            (state.identity().clone(), visible)
        };
        runtime.history_replaced(visible);
        let identity = self.identities.fresh_identity();
        if old_identity != identity {
            self.store.release(&old_identity);
        }
        runtime.session_key_changed(&identity);
        let (ledger, retention) = {
            let mut state = self.state.write().await;
            let spill_store = state.conversation().spill_store().cloned();
            let ledger = state.switch_to(identity.clone(), spill_store, messages);
            (ledger, retention_of(&state))
        };
        runtime.reset_effort_to_default();
        runtime.reset_workflow();
        clear_retention_best_effort(retention, "new_session").await;
        Ok(FreshConversationStarted { identity, ledger })
    }
}

impl std::fmt::Debug for StartFreshConversation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartFreshConversation")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "start_fresh_conversation_tests.rs"]
mod tests;
