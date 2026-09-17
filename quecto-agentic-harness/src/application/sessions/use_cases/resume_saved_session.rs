//! Resume saved session (#1863, D8 #1977): the `resume_session`
//! transaction of a loop, run after the interface admitted the request
//! (the agent is idle), and the open of the loop's own session at startup.
//! Distinct from the fresh session (#1862, D7): a named target is admitted,
//! claimed and loaded, and its history and workflow are restored.
//!
//! The transaction owns the baseline order exactly and decides every
//! step: an ephemeral loop resumes nothing; the target is admitted by the
//! affirmative rules of [`ResumeTarget`]; the departing session's children
//! are settled and the departing session saved before anything is
//! replaced (#1938); the target is claimed before it is read (#1460) and
//! that claim is released on every failure after it — a missing target, a
//! load error, a roster that cannot be replaced; the loaded session's
//! persisted child rows are history only (#1937) and the roster is
//! replaced with zero operational rows; the store's ownership of the old
//! identity is released immediately after the active key is replaced
//! (commit), when the two differ; the tracker, the agent and its tools
//! adopt the new key; the session-scoped effort is reset; the conversation
//! and the persisted watermark become the loaded history and the workflow
//! run is restored (or reset when none was saved); the injected prompt is
//! re-injected; the runtime's turn accounting is reset to the visible
//! count; and the ledger, identity and retention namespace are replaced in
//! one write, so the snapshot and the spill key follow the target
//! atomically. Every refusal precedes the key replacement and keeps the
//! current session whole; children the fleet already settled stay settled.
use crate::application::sessions::session_home::SessionHomeContext;
use std::sync::Arc;

use super::clear_conversation::visible_message_count;
use super::{DepartingChildren, SaveSession};
use crate::application::sessions::active_session::ActiveSessionHandle;
use crate::application::sessions::dto::{
    ResumeSavedSessionError, ResumeTarget, SaveTrigger, SavedSessionResumed, SessionTransition,
    StartupSessionOpened,
};
use crate::application::sessions::ports::{FleetSettlement, SessionStore, SessionSwitchRuntime};
use crate::domain::conversation_view::inject_system_prompt;
use crate::domain::message::Message;
use crate::domain::session::Session;

pub struct ResumeSavedSession {
    state: ActiveSessionHandle,
    save: Arc<SaveSession>,
    store: Arc<dyn SessionStore>,
    children: Arc<DepartingChildren>,
    /// A `--no-session` loop: nothing is claimed, loaded or resumed.
    ephemeral: bool,
    /// Scope admission (#2009) is mandatory: no loop resumes without it.
    home: SessionHomeContext,
}

#[path = "resume_saved_session_admission.rs"]
mod resume_saved_session_admission;
use resume_saved_session_admission::{PendingClaim, admit_home, admit_new_at_startup};

impl ResumeSavedSession {
    pub fn new(
        state: ActiveSessionHandle,
        save: Arc<SaveSession>,
        store: Arc<dyn SessionStore>,
        children: Arc<DepartingChildren>,
        ephemeral: bool,
        home: SessionHomeContext,
    ) -> Self {
        Self {
            state,
            save,
            store,
            children,
            ephemeral,
            home,
        }
    }

    /// Leave the current session for the saved session `raw_target` names.
    /// `messages` is the loop's live conversation; `fleet` the loop's fleet
    /// teardown, if it has one; `runtime` the loop runtime the switch moves.
    pub async fn execute(
        &self,
        raw_target: &str,
        messages: &mut Vec<Message>,
        fleet: Option<&dyn FleetSettlement>,
        runtime: &mut dyn SessionSwitchRuntime,
    ) -> Result<SavedSessionResumed, ResumeSavedSessionError> {
        if self.ephemeral {
            return Err(ResumeSavedSessionError::Ephemeral);
        }
        let target = ResumeTarget::parse(raw_target)?;
        let transition = SessionTransition::Resume;
        self.children
            .settle(fleet, transition)
            .await
            .map_err(ResumeSavedSessionError::Refused)?;
        self.save
            .save(messages, SaveTrigger::Routine)
            .await
            .map_err(ResumeSavedSessionError::Save)?;
        let old_identity = self.state.read().await.identity().clone();
        self.store
            .claim(&target.identity)
            .map_err(ResumeSavedSessionError::Claim)?;
        let mut claim = PendingClaim::new(self.store.clone(), target.identity.clone());
        claim.release = old_identity != target.identity;
        let loaded = self.load_claimed(&target).await?;
        self.children
            .note_persisted_rows_are_history(loaded.subagent_roster.len());
        if let Err(refused) = self.children.reset_roster(transition) {
            return Err(ResumeSavedSessionError::Refused(refused));
        }
        claim.release = false;
        if old_identity != target.identity {
            self.store.release(&old_identity);
        }
        runtime.session_key_changed(&target.identity);
        runtime.reset_effort_to_default();
        *messages = loaded.messages;
        let injected_prompt = {
            let mut state = self.state.write().await;
            state.set_persisted_watermark(messages.len());
            state.injected_system_prompt().to_string()
        };
        match loaded.workflow_run {
            Some(run) => runtime.restore_workflow(run),
            None => runtime.reset_workflow(),
        }
        inject_system_prompt(messages, &injected_prompt);
        runtime.history_replaced(visible_message_count(messages, &injected_prompt));
        let ledger = {
            let mut state = self.state.write().await;
            let spill_store = state.conversation().spill_store().cloned();
            state.switch_to(target.identity.clone(), spill_store, messages)
        };
        Ok(SavedSessionResumed {
            name: target.name,
            identity: target.identity,
            message_count: messages.len(),
            ledger,
        })
    }

    /// Read the claimed target; the caller releases the claim on `Err`.
    async fn load_claimed(
        &self,
        target: &ResumeTarget,
    ) -> Result<Session, ResumeSavedSessionError> {
        match self.store.load(&target.identity).await {
            Ok(Some(session)) => {
                admit_home(&self.home, &target.identity).await?;
                Ok(session)
            }
            Ok(None) => Err(ResumeSavedSessionError::NotFound(target.name.clone())),
            Err(err) => Err(ResumeSavedSessionError::Load(err)),
        }
    }

    /// Open the session the loop was composed on, at startup: claim it
    /// (#1460 — a key owned by another live process is refused before any
    /// turn runs against it), load what the store holds, and let the
    /// persisted watermark stand for it. An ephemeral run, or the ephemeral
    /// identity, touches the store not at all. A load failure releases the
    /// claim just taken.
    pub async fn open_at_startup(&self) -> Result<StartupSessionOpened, ResumeSavedSessionError> {
        let identity = self.state.read().await.identity().clone();
        let session = if self.ephemeral || identity.is_ephemeral() {
            Session::new(identity)
        } else {
            self.store
                .claim(&identity)
                .map_err(ResumeSavedSessionError::Claim)?;
            let mut claim = PendingClaim::new(self.store.clone(), identity.clone());
            let session = match self.store.load(&identity).await {
                Ok(Some(session)) => {
                    admit_home(&self.home, &identity).await?;
                    session
                }
                Ok(None) => {
                    admit_new_at_startup(&self.home, &identity).await?;
                    Session::new(identity.clone())
                }
                Err(err) => {
                    return Err(ResumeSavedSessionError::Load(err));
                }
            };
            claim.release = false;
            session
        };
        self.state
            .write()
            .await
            .set_persisted_watermark(session.messages.len());
        Ok(StartupSessionOpened {
            messages: session.messages,
            workflow_run: session.workflow_run,
        })
    }
}

impl std::fmt::Debug for ResumeSavedSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResumeSavedSession")
            .field("ephemeral", &self.ephemeral)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "resume_saved_session_tests.rs"]
mod tests;
