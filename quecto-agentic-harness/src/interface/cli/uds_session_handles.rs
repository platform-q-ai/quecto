//! What a dispatch loop needs from the sessions capability (#1970, #1971,
//! #1974), as plain handles.
//!
//! The interface declares the runtime inputs one loop hands over and the
//! store, state and controller handles it holds back; composition owns the
//! concrete graph between them (`composition::sessions`) and hands its
//! builder in through [`crate::interface::cli::CliContext`] as a
//! [`crate::interface::cli::SessionHandlesBuilder`], so no interface module
//! ever names the composition layer, constructs a store, a use case or the
//! active-session state, or forms a path.
use std::path::PathBuf;
use std::sync::Arc;

use crate::application::sessions::active_session::ActiveSessionHandle;
use crate::application::sessions::ports::{ContextSpillStore, SessionStore};
use crate::interface::uds::sessions::controller::ListSessionsController;
use crate::interface::uds::sessions::export_report_controller::ExportSessionReportController;
use crate::interface::uds::sessions::read_history_controller::ReadHistoryController;
use crate::interface::uds::sessions::recover_message_controller::RecoverMessageController;

/// The runtime inputs of one loop the session handles are composed over.
pub struct SessionLoopInputs {
    /// The harness base directory the file store lives under.
    pub base_dir: PathBuf,
    /// A store the loop already holds (unit rigs, BDD fixtures); `None`
    /// composes the file store of `base_dir`.
    pub store: Option<Arc<dyn SessionStore>>,
    /// The raw session key the loop was opened on (empty for an ephemeral
    /// run): the persisted key a resumed or named session was admitted
    /// under, or the fresh chat key generated at startup.
    pub session_key: String,
    /// The retention store of the loop's agent, paired with the active
    /// session for collapsed-message recovery.
    pub spill_store: Option<Arc<dyn ContextSpillStore>>,
}

/// The handles one loop holds on the sessions capability.
pub struct SessionHandles {
    /// The session store every session transaction of the loop runs
    /// against.
    pub store: Arc<dyn SessionStore>,
    /// List saved sessions (#1861): the UDS `list_sessions` command.
    pub list_sessions: Arc<ListSessionsController>,
    /// The one active session of the loop (#1971): typed identity and the
    /// live-conversation read model every transport reads.
    pub active_session: ActiveSessionHandle,
    /// Read conversation history (#1856): `get_messages` and its alias.
    pub read_history: Arc<ReadHistoryController>,
    /// Recover full message/tool-call content (#1858): `get_message`.
    pub recover_message: Arc<RecoverMessageController>,
    /// Export a retained session report (#1859): `get_report`, exporting
    /// under the root composition supplied.
    pub export_report: Arc<ExportSessionReportController>,
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
        }
    }
}

/// The active session and its read use cases, cloned into every task that
/// serves reads while the dispatch loop is busy.
#[derive(Clone)]
pub struct SessionReadHandles {
    pub active_session: ActiveSessionHandle,
    pub read_history: Arc<ReadHistoryController>,
    pub recover_message: Arc<RecoverMessageController>,
    pub export_report: Arc<ExportSessionReportController>,
}

impl std::fmt::Debug for SessionReadHandles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionReadHandles").finish_non_exhaustive()
    }
}
