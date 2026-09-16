//! Retained-context composition (#1866, D9 #1978): the one recall use case
//! and the narrow writer/reader pair of the sessions capability, built over
//! one retention store. The file store itself is composed beside the
//! session store, over the same flat layout
//! (`composition::sessions::build_retention_handles`); this module builds
//! the graph over whatever store it is handed, so unit rigs and BDD
//! fixtures compose the same graph over an in-memory store.
use std::sync::Arc;

use crate::application::context::ContextRetention;
use crate::application::sessions::ports::ContextSpillStore;
use crate::application::sessions::use_cases::{ListRetainedContext, RecallContext, RetainContext};
use crate::interface::cli::retention_handles::RetentionHandles;

/// The retention handles of one run over `store`.
pub fn retention_handles_over(store: Arc<dyn ContextSpillStore>) -> RetentionHandles {
    RetentionHandles {
        recall: Arc::new(RecallContext::new(store.clone())),
        context: context_retention_over(store.clone()),
        store,
    }
}

/// The pruning policy's writer/reader pair over `store` (unit rigs and BDD
/// fixtures that drive an agent loop alone compose exactly this).
pub fn context_retention_over(store: Arc<dyn ContextSpillStore>) -> ContextRetention {
    ContextRetention {
        retain: Arc::new(RetainContext::new(store.clone())),
        list: Arc::new(ListRetainedContext::new(store)),
    }
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod tests;
