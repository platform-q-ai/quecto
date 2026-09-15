//! The active-session graph of one harness loop (#1971–#1976): the one
//! application-owned `ActiveSessionState` (R7a), the live-conversation
//! read, sync and report use cases, the save transaction, the clear and
//! rewind transactions and the fresh-session transaction over it, and the
//! loop's session handles around them. The loop's raw session key
//! becomes the typed identity here: the exact persisted-key round-trip,
//! the only conversion outside persistence. The runtime sources the save
//! transaction snapshots — the workflow engine and the sub-agent registry
//! — and the roster the transitions count and replace are adapted here;
//! the agent's dirty latch is a port on its own, and the fleet teardown
//! is the loop's late-bound handle, passed to the transaction per call.
use std::sync::Arc;

use crate::application::sessions::active_session::{ActiveSessionHandle, ActiveSessionState};
use crate::application::sessions::ports::export::SessionExportPort;
use crate::application::sessions::ports::{
    DelegatedChildrenRoster, FreshSessionIdentityGenerator, HistoricalRosterSource, SessionStore,
    WorkflowRunSource,
};
use crate::application::sessions::use_cases::{
    ClearConversation, DepartingChildren, ReadHistory, RecoverMessage, RewindConversation,
    SaveSession, StartFreshConversation, SynchronizeTranscript,
};
use crate::domain::session_identity::SessionIdentity;
use crate::infrastructure::persistence::session_snapshot_sources::{
    RegistryRosterSource, WorkflowEngineRunSource,
};
use crate::infrastructure::tools::delegated_roster::RegistryDelegatedRoster;
use crate::interface::cli::uds_session_handles::{
    ConversationRewriteHandles, SessionHandles, SessionLoopInputs, SessionSwitchHandles,
};
use crate::interface::uds::sessions::controller::ListSessionsController;
use crate::interface::uds::sessions::read_history_controller::ReadHistoryController;
use crate::interface::uds::sessions::recover_message_controller::RecoverMessageController;
use crate::interface::uds::sessions::synchronize_transcript_controller::SynchronizeTranscriptController;

/// The handles of a loop opened on `inputs.session_key` with the loop's
/// retention backstop, injected prompt, dirty latch, workflow and roster
/// runtime: the one active-session state, the history, recovery, sync,
/// report, save, clear, rewind and fresh-session use cases over it,
/// `store`, `export` and `identities`, and the list controller the
/// sessions composition already built.
pub fn assemble_session_handles(
    inputs: SessionLoopInputs,
    store: Arc<dyn SessionStore>,
    list_sessions: Arc<ListSessionsController>,
    export: Option<Arc<dyn SessionExportPort>>,
    identities: Arc<dyn FreshSessionIdentityGenerator>,
) -> SessionHandles {
    let mut state =
        ActiveSessionState::new(SessionIdentity::from_persisted_key(inputs.session_key));
    state.set_spill_store(inputs.spill_store);
    state.set_injected_system_prompt(inputs.system_prompt);
    let active_session: ActiveSessionHandle = Arc::new(tokio::sync::RwLock::new(state));
    let read_history = Arc::new(ReadHistory::new(active_session.clone(), store.clone()));
    let recover_message = Arc::new(RecoverMessage::new(active_session.clone()));
    let export_report = super::session_report::build_export_report(active_session.clone(), export);
    let synchronize = Arc::new(SynchronizeTranscript::new(active_session.clone()));
    let workflow = inputs
        .workflow_state
        .map(|engine| Arc::new(WorkflowEngineRunSource::new(engine)) as Arc<dyn WorkflowRunSource>);
    let roster = inputs.subagent_registry.clone().map(|registry| {
        Arc::new(RegistryRosterSource::new(registry)) as Arc<dyn HistoricalRosterSource>
    });
    let delegated = inputs.subagent_registry.map(|registry| {
        Arc::new(RegistryDelegatedRoster::new(registry)) as Arc<dyn DelegatedChildrenRoster>
    });
    let save_session = Arc::new(SaveSession::new(
        active_session.clone(),
        store.clone(),
        inputs.durable_prefix,
        workflow,
        roster,
        inputs.ephemeral,
    ));
    let rewrite = ConversationRewriteHandles {
        clear: Arc::new(ClearConversation::new(
            active_session.clone(),
            save_session.clone(),
        )),
        rewind: Arc::new(RewindConversation::new(
            active_session.clone(),
            save_session.clone(),
        )),
    };
    let departing_children = Arc::new(DepartingChildren::new(delegated));
    let switch = SessionSwitchHandles {
        fresh: Arc::new(StartFreshConversation::new(
            active_session.clone(),
            save_session.clone(),
            store.clone(),
            identities,
            departing_children.clone(),
        )),
        departing_children,
    };
    SessionHandles {
        store,
        list_sessions,
        active_session,
        read_history: Arc::new(ReadHistoryController::new(read_history)),
        recover_message: Arc::new(RecoverMessageController::new(recover_message)),
        save_session,
        rewrite,
        switch,
        export_report,
        synchronize_transcript: Arc::new(SynchronizeTranscriptController::new(synchronize)),
    }
}

#[cfg(test)]
#[path = "active_session_tests.rs"]
mod tests;
