//! Application use case for publishing a catalogue snapshot together with its
//! runtime provider.
//!
//! Runtime composition is separate from provider-construction wiring: callers
//! hand this use case an already-resolved immutable catalogue snapshot plus an
//! application port that can compose the runtime for that exact generation.

use std::sync::Arc;

use crate::domain::catalogue::CatalogueSnapshot;
use crate::domain::provider::LlmProvider;

#[derive(Debug, Clone)]
pub struct CatalogueRuntimeSnapshot {
    pub catalogue: CatalogueSnapshot,
    pub provider: Arc<dyn LlmProvider>,
}

impl CatalogueRuntimeSnapshot {
    pub fn generation(&self) -> u64 {
        self.catalogue.generation
    }
}
