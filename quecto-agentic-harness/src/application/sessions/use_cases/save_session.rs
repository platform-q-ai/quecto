//! Save current session (#1860): the one transaction every persistence
//! trigger of a loop runs — after a turn, before a turn's user prompt runs,
//! on an explicit `persist_session`, around a session transition and on
//! the loop's ordinary exit.
//!
//! The transaction owns the order and every state change: it strips the
//! injected system prompt from what is written and re-injects it after,
//! assigns the durable ordinals, drains the agent's sticky dirty latch into
//! the session state, resets the watermark when history shrank, normalises
//! the restore reason (an unknown reason is the legacy one; an armed
//! killing exit wins), snapshots the workflow run and the historical
//! roster (empty on a killing exit), chooses a full save or a clean delta,
//! and advances the watermark and drains the latch only once the store
//! succeeded. The store performs locking, atomic replacement and fsync;
//! the ports are observations of the runtime, never mutations.
//!
//! One transaction runs at a time per loop: the barrier serialises
//! concurrent requests so a later save always sees the watermark the
//! earlier one committed. Readers of the active session are never blocked
//! on store I/O — the state lock is held only to read inputs and to commit.
use std::sync::Arc;

use crate::application::sessions::active_session::ActiveSessionHandle;
use crate::application::sessions::dto::{SaveMode, SaveOutcome, SaveSessionError, SaveTrigger};
use crate::application::sessions::ports::{
    DurablePrefixObservation, HistoricalRosterSource, SessionStore, WorkflowRunSource,
};
use crate::domain::conversation_view::{inject_system_prompt, remove_injected_system_prompt};
use crate::domain::message::Message;
use crate::domain::session::{
    PersistedSubagentRosterEntry, Session, SubagentRestoreReason, assign_missing_ordinals,
};
use crate::domain::session_identity::SessionIdentity;

pub struct SaveSession {
    state: ActiveSessionHandle,
    store: Arc<dyn SessionStore>,
    durable_prefix: Arc<dyn DurablePrefixObservation>,
    workflow: Option<Arc<dyn WorkflowRunSource>>,
    /// `None` when the loop tracks no sub-agent roster: a clean delta may
    /// then omit the roster; a tracked roster is always written whole.
    roster: Option<Arc<dyn HistoricalRosterSource>>,
    /// A `--no-session` run: every save is affirmatively a no-op.
    ephemeral: bool,
    barrier: tokio::sync::Mutex<()>,
}

/// The inputs read from the active session under one short lock.
struct SaveInputs {
    identity: SessionIdentity,
    injected_prompt: String,
    watermark: usize,
    killing_exit: bool,
}

impl SaveSession {
    pub fn new(
        state: ActiveSessionHandle,
        store: Arc<dyn SessionStore>,
        durable_prefix: Arc<dyn DurablePrefixObservation>,
        workflow: Option<Arc<dyn WorkflowRunSource>>,
        roster: Option<Arc<dyn HistoricalRosterSource>>,
        ephemeral: bool,
    ) -> Self {
        Self {
            state,
            store,
            durable_prefix,
            workflow,
            roster,
            ephemeral,
            barrier: tokio::sync::Mutex::new(()),
        }
    }

    /// Persist `messages` (the loop's live conversation, injected prompt
    /// included) as the current session.
    pub async fn save(
        &self,
        messages: &mut Vec<Message>,
        trigger: SaveTrigger,
    ) -> Result<SaveOutcome, SaveSessionError> {
        let _serialised = self.barrier.lock().await;
        let Some(inputs) = self.begin(trigger).await else {
            return Ok(SaveOutcome::Ephemeral);
        };
        remove_injected_system_prompt(messages, &inputs.injected_prompt);
        assign_missing_ordinals(messages);
        // Drain the agent's latch into the session state before the store is
        // touched: a failed or cancelled save keeps the observation and the
        // reset watermark, so the next save reconciles instead of appending
        // against a prefix that changed.
        let taken = self.durable_prefix.take_durable_prefix_dirty();
        let shrank = messages.len() < inputs.watermark;
        let dirty = {
            let mut state = self.state.write().await;
            if taken {
                state.latch_durable_prefix_dirty();
            }
            if shrank {
                state.set_persisted_watermark(0);
            }
            state.durable_prefix_dirty()
        };
        let watermark = if shrank { 0 } else { inputs.watermark };
        let workflow_run = self.workflow.as_ref().and_then(|w| w.persisted_run());
        let restore_reason = effective_restore_reason(trigger, inputs.killing_exit);
        let roster = self.roster.as_ref().map(|source| {
            if restore_reason == SubagentRestoreReason::OrdinaryTuiExitStopped {
                // A killing exit preserves history, never an operational
                // child roster.
                Vec::new()
            } else {
                historical_roster(source.roster_rows(), restore_reason)
            }
        });
        let mode = if trigger.forces_full_save() || dirty || roster.is_some() {
            SaveMode::Full
        } else {
            SaveMode::CleanDelta
        };
        let result = match mode {
            SaveMode::Full => {
                self.store
                    .save(&Session {
                        key: inputs.identity,
                        messages: messages.clone(),
                        workflow_run,
                        subagent_roster: roster.unwrap_or_default(),
                    })
                    .await
            }
            SaveMode::CleanDelta => {
                self.store
                    .save_clean_delta(&inputs.identity, messages, watermark, workflow_run)
                    .await
            }
        };
        let persisted = messages.len();
        inject_system_prompt(messages, &inputs.injected_prompt);
        result.map_err(SaveSessionError::Store)?;
        self.commit(persisted).await;
        Ok(SaveOutcome::Saved { mode, persisted })
    }

    /// Persist the conversation with `pending` — the user prompt about to
    /// run — appended, before the turn starts, so the prompt survives an
    /// ungraceful exit mid-turn. `pending` receives its durable ordinal;
    /// `messages` is left as the live conversation. The agent's latch is
    /// not drained here: the durable prefix is verified by the store.
    pub async fn save_with_pending_prompt(
        &self,
        messages: &[Message],
        pending: &mut Message,
    ) -> Result<SaveOutcome, SaveSessionError> {
        let _serialised = self.barrier.lock().await;
        let Some(inputs) = self.begin(SaveTrigger::Routine).await else {
            return Ok(SaveOutcome::Ephemeral);
        };
        let mut persisted = messages.to_vec();
        remove_injected_system_prompt(&mut persisted, &inputs.injected_prompt);
        persisted.push(pending.clone());
        assign_missing_ordinals(&mut persisted);
        pending.ordinal = persisted.last().and_then(|message| message.ordinal);
        let workflow_run = self.workflow.as_ref().and_then(|w| w.persisted_run());
        let mode = if self.roster.is_some() {
            SaveMode::Full
        } else {
            SaveMode::CleanDelta
        };
        let result = match &self.roster {
            Some(source) => {
                self.store
                    .save(&Session {
                        key: inputs.identity,
                        subagent_roster: historical_roster(
                            source.roster_rows(),
                            SubagentRestoreReason::LegacyUnspecified,
                        ),
                        messages: persisted.clone(),
                        workflow_run,
                    })
                    .await
            }
            None => {
                self.store
                    .save_delta(&inputs.identity, &persisted, inputs.watermark, workflow_run)
                    .await
            }
        };
        result.map_err(SaveSessionError::Store)?;
        let persisted = persisted.len();
        self.state.write().await.set_persisted_watermark(persisted);
        Ok(SaveOutcome::Saved { mode, persisted })
    }

    /// Read the transaction's inputs and move the killing-exit state an
    /// explicit request carries; `None` when there is nothing to save.
    async fn begin(&self, trigger: SaveTrigger) -> Option<SaveInputs> {
        let mut state = self.state.write().await;
        if self.ephemeral || state.identity().is_ephemeral() {
            return None;
        }
        if let SaveTrigger::Explicit { restore_reason } = trigger {
            // Explicit detach persistence can cancel a prior killing request;
            // routine saves cannot. The intent lives in memory, not in
            // obsolete historical rows.
            state.set_killing_exit(restore_reason == SubagentRestoreReason::OrdinaryTuiExitStopped);
        }
        Some(SaveInputs {
            identity: state.identity().clone(),
            injected_prompt: state.injected_system_prompt().to_string(),
            watermark: state.persisted_watermark(),
            killing_exit: state.killing_exit(),
        })
    }

    /// The store succeeded: the whole conversation is durable.
    async fn commit(&self, persisted: usize) {
        let mut state = self.state.write().await;
        state.set_persisted_watermark(persisted);
        state.clear_durable_prefix_dirty();
    }
}

/// The reason the persisted rows carry: an unknown wire reason is the
/// legacy one, and an armed killing exit takes precedence over any reason.
fn effective_restore_reason(trigger: SaveTrigger, killing_exit: bool) -> SubagentRestoreReason {
    if killing_exit {
        return SubagentRestoreReason::OrdinaryTuiExitStopped;
    }
    match trigger {
        SaveTrigger::Explicit {
            restore_reason: SubagentRestoreReason::Unknown,
        }
        | SaveTrigger::Routine
        | SaveTrigger::OrdinaryExit => SubagentRestoreReason::LegacyUnspecified,
        SaveTrigger::Explicit { restore_reason } => restore_reason,
    }
}

/// The rows as the session records them: stamped with `restore_reason`
/// and ordered by identity so the file is deterministic.
fn historical_roster(
    mut rows: Vec<PersistedSubagentRosterEntry>,
    restore_reason: SubagentRestoreReason,
) -> Vec<PersistedSubagentRosterEntry> {
    for row in &mut rows {
        row.restore_reason = restore_reason;
    }
    rows.sort_by(|a, b| a.agent_uuid.cmp(&b.agent_uuid));
    rows
}

impl std::fmt::Debug for SaveSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveSession")
            .field("ephemeral", &self.ephemeral)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "save_session_rig_tests.rs"]
mod rig_tests;
#[cfg(test)]
#[path = "save_session_tests.rs"]
mod tests;
