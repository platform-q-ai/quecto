//! Sessions composition (#1970–#1976): the flat layout, the file store over
//! it, the export root, the fresh-identity generator, the list use case
//! and, through `composition::active_session`, the active session and its
//! use cases.
use crate::application::sessions::ports::{FreshSessionIdentityGenerator, SessionStore};
use crate::application::sessions::use_cases::ListSessions;
use crate::infrastructure::persistence::fresh_session_identity::ProcessClockIdentityGenerator;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::interface::uds::sessions::controller::ListSessionsController;
use std::sync::Arc;

pub use crate::interface::cli::uds_session_handles::{SessionHandles, SessionLoopInputs};

/// The generator of fresh user-chat identities (D7 #1976): the process
/// clock and counter adapter, shared by the startup key of a chat run and
/// by every fresh-session transaction of the process.
pub fn build_fresh_session_identity() -> Arc<dyn FreshSessionIdentityGenerator> {
    Arc::new(ProcessClockIdentityGenerator::new())
}

/// The file session store of one base directory: the one flat layout and
/// the record adapter over it.
pub fn build_file_session_store(base_dir: &std::path::Path) -> FileSessionStore {
    FileSessionStore::new(FlatSessionLayout::new(base_dir))
}

/// The handles one loop holds, over the file store of `inputs.base_dir`
/// unless the loop supplied a store (unit rigs and BDD fixtures).
pub fn build_session_handles(mut inputs: SessionLoopInputs) -> SessionHandles {
    let store: Arc<dyn SessionStore> = inputs
        .store
        .take()
        .unwrap_or_else(|| Arc::new(build_file_session_store(&inputs.base_dir)));
    let list_sessions = Arc::new(ListSessions::new(store.clone()));
    let export = super::session_report::build_session_export(&inputs.base_dir);
    super::active_session::assemble_session_handles(
        inputs,
        store,
        Arc::new(ListSessionsController::new(list_sessions)),
        Some(export),
        build_fresh_session_identity(),
    )
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
