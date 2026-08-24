//! Application use case for publishing a catalogue snapshot together with its
//! runtime provider.
//!
//! Runtime composition is separate from provider-construction wiring: callers
//! hand this use case an already-resolved immutable catalogue snapshot plus an
//! application port that can compose the runtime for that exact generation.

use std::sync::Arc;

use crate::domain::catalogue::CatalogueSnapshot;
use crate::domain::provider::LlmProvider;

pub trait CatalogueRuntimeComposer {
    fn compose(&self, snapshot: &CatalogueSnapshot) -> Result<Arc<dyn LlmProvider>, String>;
}

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

pub struct ComposeCatalogueRuntimeUseCase<'a> {
    composer: &'a dyn CatalogueRuntimeComposer,
}

impl<'a> ComposeCatalogueRuntimeUseCase<'a> {
    pub fn new(composer: &'a dyn CatalogueRuntimeComposer) -> Self {
        Self { composer }
    }

    pub fn compose(&self, snapshot: CatalogueSnapshot) -> Result<CatalogueRuntimeSnapshot, String> {
        let provider = self.composer.compose(&snapshot)?;
        Ok(CatalogueRuntimeSnapshot {
            catalogue: snapshot,
            provider,
        })
    }
}

#[cfg(test)]
#[path = "catalogue_runtime_tests.rs"]
mod tests;
