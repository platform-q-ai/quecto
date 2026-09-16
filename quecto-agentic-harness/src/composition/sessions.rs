use crate::application::sessions::ports::{FreshSessionIdentityGenerator, SessionStore};
use crate::application::sessions::use_cases::ListSessions;
use crate::infrastructure::persistence::{
    context_spill::FileContextSpillStore, fresh_session_identity::ProcessClockIdentityGenerator,
    session_layout::FlatSessionLayout, session_store::FileSessionStore,
};
pub use crate::interface::cli::uds_session_handles::{SessionHandles, SessionLoopInputs};
use crate::interface::{
    cli::retention_handles::RetentionHandles, uds::sessions::controller::ListSessionsController,
};
use std::sync::Arc;
pub fn build_fresh_session_identity() -> Arc<dyn FreshSessionIdentityGenerator> {
    Arc::new(ProcessClockIdentityGenerator::new())
}
pub fn build_file_session_store(base_dir: &std::path::Path) -> FileSessionStore {
    FileSessionStore::new(FlatSessionLayout::new(base_dir))
}
pub fn build_retention_handles(base_dir: &std::path::Path) -> RetentionHandles {
    super::retention::retention_handles_over(Arc::new(FileContextSpillStore::new(
        FlatSessionLayout::new(base_dir),
    )))
}
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
mod resume_factory;
use resume_factory as resume;
#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
pub use resume::production_resume_handle;
