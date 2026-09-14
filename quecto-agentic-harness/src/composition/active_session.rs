//! The active-session graph of one harness loop (#1971): the one
//! application-owned `ActiveSessionState` (R7a), the live-conversation
//! read use cases over it, and the assembly of the loop's session handles
//! around them. The loop's raw session key becomes the typed identity
//! here: the exact persisted-key round-trip, the only conversion outside
//! persistence.
use std::sync::Arc;

use crate::application::sessions::active_session::{ActiveSessionHandle, ActiveSessionState};
use crate::application::sessions::ports::{ContextSpillStore, SessionStore};
use crate::application::sessions::use_cases::{ReadHistory, RecoverMessage};
use crate::domain::session_identity::SessionIdentity;
use crate::interface::cli::uds_session_handles::SessionHandles;
use crate::interface::uds::sessions::controller::ListSessionsController;
use crate::interface::uds::sessions::read_history_controller::ReadHistoryController;
use crate::interface::uds::sessions::recover_message_controller::RecoverMessageController;

/// The handles of a loop opened on `session_key` with `spill_store` as its
/// retention backstop: the one active-session state, the history and
/// recovery use cases over it and `store`, and the list controller the
/// sessions composition already built.
pub fn assemble_session_handles(
    session_key: String,
    spill_store: Option<Arc<dyn ContextSpillStore>>,
    store: Arc<dyn SessionStore>,
    list_sessions: Arc<ListSessionsController>,
) -> SessionHandles {
    let mut state = ActiveSessionState::new(SessionIdentity::from_persisted_key(session_key));
    state.set_spill_store(spill_store);
    let active_session: ActiveSessionHandle = Arc::new(tokio::sync::RwLock::new(state));
    let read_history = Arc::new(ReadHistory::new(active_session.clone(), store.clone()));
    let recover_message = Arc::new(RecoverMessage::new(active_session.clone()));
    SessionHandles {
        store,
        list_sessions,
        active_session,
        read_history: Arc::new(ReadHistoryController::new(read_history)),
        recover_message: Arc::new(RecoverMessageController::new(recover_message)),
    }
}

#[cfg(test)]
#[path = "active_session_tests.rs"]
mod tests;
