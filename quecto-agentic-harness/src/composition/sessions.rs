//! The concrete graph of the sessions capability for one harness (#1970):
//! the flat layout, the file session store over it, and the use cases the
//! interface holds as handles. The interface declares the loop inputs and
//! the handles it holds (`interface::cli::uds_session_handles`) and receives
//! this builder through its `CliContext`; it never names this module.
use std::sync::Arc;

use crate::application::sessions::ports::SessionStore;
use crate::application::sessions::use_cases::ListSessions;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::interface::uds::sessions::controller::ListSessionsController;

pub use crate::interface::cli::uds_session_handles::{SessionHandles, SessionLoopInputs};

/// The file session store of one base directory: the one flat layout and
/// the record adapter over it.
pub fn build_file_session_store(base_dir: &std::path::Path) -> FileSessionStore {
    FileSessionStore::new(FlatSessionLayout::new(base_dir))
}

/// The handles one loop holds on the sessions capability, over the file
/// store of `inputs.base_dir` unless the loop supplied a store (unit rigs
/// and BDD fixtures).
pub fn build_session_handles(inputs: SessionLoopInputs) -> SessionHandles {
    let store: Arc<dyn SessionStore> = inputs
        .store
        .unwrap_or_else(|| Arc::new(build_file_session_store(&inputs.base_dir)));
    let list_sessions = Arc::new(ListSessions::new(store.clone()));
    SessionHandles {
        store,
        list_sessions: Arc::new(ListSessionsController::new(list_sessions)),
    }
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
