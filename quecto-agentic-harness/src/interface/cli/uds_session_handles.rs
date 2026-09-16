//! What a dispatch loop needs from the sessions capability (#1968), as
//! plain handles: the interface declares the typed runtime inputs one loop
//! hands over and the store, state, use-case and controller handles it
//! holds back; composition owns the graph between them and hands its
//! builder in through [`crate::interface::cli::CliContext`] as a
//! [`crate::interface::cli::SessionHandlesBuilder`], so no interface module
//! names the composition layer, constructs a store, a use case or the
//! active-session state, converts a raw key, or forms a path.
use std::path::PathBuf;
use std::sync::Arc;

use crate::application::durable_prefix::DurablePrefixLatch;
use crate::application::sessions::active_session::ActiveSessionHandle;
use crate::application::sessions::ports::{ContextSpillStore, SessionStore};
use crate::application::sessions::use_cases::{
    ClearConversation, ResumeSavedSession, RewindConversation, SaveSession, StartFreshConversation,
};
use crate::domain::session_identity::SessionIdentity;
use crate::interface::uds::sessions::controller::ListSessionsController;
use crate::interface::uds::sessions::export_report_controller::ExportSessionReportController;
use crate::interface::uds::sessions::read_history_controller::ReadHistoryController;
use crate::interface::uds::sessions::recover_message_controller::RecoverMessageController;
use crate::interface::uds::sessions::synchronize_transcript_controller::SynchronizeTranscriptController;

/// The runtime inputs of one loop the session handles are composed over.
pub struct SessionLoopInputs {
    /// The harness base directory the file store lives under.
    pub base_dir: PathBuf,
    /// A store the loop already holds (rigs); `None` composes the file store.
    pub store: Option<Arc<dyn SessionStore>>,
    /// The typed identity the loop was opened on (D10 #1979): ephemeral,
    /// the named `cli:<name>`, or the fresh chat identity drawn at startup.
    pub identity: SessionIdentity,
    /// A `--no-session` run (#1860): the save transaction is a no-op.
    pub ephemeral: bool,
    /// The system prompt injected at the conversation head; never persisted.
    pub system_prompt: String,
    /// The loop agent's retention store, for collapsed-message recovery.
    pub spill_store: Option<Arc<dyn ContextSpillStore>>,
    /// The agent's durable-prefix dirty latch (#1072) the save drains.
    pub durable_prefix: Arc<DurablePrefixLatch>,
    /// The bound workflow engine whose run the session records, if any.
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>,
    /// The sub-agent registry whose rows the session records as history (#1937).
    pub subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
}

/// The handles one loop holds on the sessions capability.
pub struct SessionHandles {
    /// The session store every session transaction of the loop runs against.
    pub store: Arc<dyn SessionStore>,
    /// Scoped discovery (#2009): maps the UDS `list_sessions` command to the single query owner.
    pub list_sessions: Arc<ListSessionsController>,
    /// The one active session of the loop (#1971): typed identity, the
    /// read model every transport reads, the persistence state.
    pub active_session: ActiveSessionHandle,
    /// Read conversation history (#1856): `get_messages` and its alias.
    pub read_history: Arc<ReadHistoryController>,
    /// Recover full message/tool-call content (#1858): `get_message`.
    pub recover_message: Arc<RecoverMessageController>,
    /// Export a retained session report (#1859): `get_report`.
    pub export_report: Arc<ExportSessionReportController>,
    /// Synchronize a client transcript (#1857): `sync` on both transports.
    pub synchronize_transcript: Arc<SynchronizeTranscriptController>,
    /// Save current session (#1860): every persistence trigger of the loop.
    pub save_session: Arc<SaveSession>,
    /// Clear (#1864) and rewind (#1865): the history-replacing transactions.
    pub rewrite: ConversationRewriteHandles,
    /// Start fresh (#1862) and resume (#1863): the session transitions.
    pub switch: SessionSwitchHandles,
}

/// The session transitions (#1976, #1977): the fresh-session transaction
/// and the resume transaction, which also opens the loop's own session.
#[derive(Clone)]
pub struct SessionSwitchHandles {
    pub fresh: Arc<StartFreshConversation>,
    pub resume: Arc<ResumeSavedSession>,
}

/// The history-replacing transactions (#1975), requested once the dispatch
/// loop admitted the command; the sessions edge maps the `rewind_to` target.
#[derive(Clone)]
pub struct ConversationRewriteHandles {
    pub clear: Arc<ClearConversation>,
    pub rewind: Arc<RewindConversation>,
}

impl SessionHandles {
    /// The handles the reader-side tasks (accept loop, per-client reader,
    /// turn publisher) share with the dispatch loop.
    pub fn read_handles(&self) -> SessionReadHandles {
        SessionReadHandles {
            active_session: self.active_session.clone(),
            read_history: self.read_history.clone(),
            recover_message: self.recover_message.clone(),
            export_report: self.export_report.clone(),
            synchronize_transcript: self.synchronize_transcript.clone(),
        }
    }
}

/// The active session and its read use cases, cloned into every task that
/// serves reads while the dispatch loop is busy.
#[derive(Clone, Debug)]
pub struct SessionReadHandles {
    pub active_session: ActiveSessionHandle,
    pub read_history: Arc<ReadHistoryController>,
    pub recover_message: Arc<RecoverMessageController>,
    pub export_report: Arc<ExportSessionReportController>,
    pub synchronize_transcript: Arc<SynchronizeTranscriptController>,
}

impl SessionReadHandles {
    /// The key of the session the loop stands for, read from the active
    /// session's identity (D10 #1979): the one source of every presented
    /// `sessionKey` and of the parent identity the loop checks.
    pub async fn current_session_key(&self) -> String {
        let state = self.active_session.read().await;
        state.identity().runtime_key().to_string()
    }
}
