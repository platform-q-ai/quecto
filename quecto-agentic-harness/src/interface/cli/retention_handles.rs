//! What a harness run holds on the sessions capability's retained context
//! (#1866, D9 #1978), as plain handles: the one retention store the active
//! session recovers collapsed messages through, the recall use case the
//! `recall` tool adapts and the run-end scrub reaches, and the narrow
//! writer/reader pair the agent loop's pruning policy consumes. Composition
//! builds all of them over one store per base directory
//! (`composition::sessions::build_retention_handles`) and hands the builder
//! in through [`crate::interface::cli::CliContext`]; no interface module
//! constructs the store, a use case or the recall tool's graph.
use std::sync::Arc;

use crate::application::context::ContextRetention;
use crate::application::sessions::ports::ContextSpillStore;
use crate::application::sessions::use_cases::RecallContext;

#[derive(Clone)]
pub struct RetentionHandles {
    /// The retention store of the run's base directory: the active
    /// session's recovery backstop and the namespace the session
    /// transactions clear and switch.
    pub store: Arc<dyn ContextSpillStore>,
    /// Recall, index and lifecycle of retained context: the `recall` tool's
    /// use case and the ephemeral run-end scrub.
    pub recall: Arc<RecallContext>,
    /// The pruning policy's writer and reader.
    pub context: ContextRetention,
}

impl std::fmt::Debug for RetentionHandles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetentionHandles").finish_non_exhaustive()
    }
}
