//! Application-level provider runtime composition.
//!
//! The application owns the composition use-case seam. Concrete configuration
//! and runtime input shapes stay outside this layer: callers select those types
//! via the generic factory implementation, keeping application independent of
//! infrastructure.

use std::sync::Arc;

use crate::domain::catalogue::CatalogueSnapshot;
use crate::domain::provider::LlmProvider;

/// Provider runtime and catalogue published as one immutable generation.
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

pub trait ProviderRuntimeFactory<C, R> {
    fn compose_runtime(
        &self,
        config: &C,
        runtime_inputs: &R,
    ) -> Result<Arc<dyn LlmProvider>, String>;
}

/// Compose provider routing and its catalogue from exactly the same adapter result.
pub fn compose_catalogue_runtime<C, R, F: ProviderRuntimeFactory<C, R>>(
    factory: &F,
    config: &C,
    runtime_inputs: &R,
    generation: u64,
) -> Result<CatalogueRuntimeSnapshot, String> {
    let provider = ComposeProviderRuntimeUseCase::new().compose(factory, config, runtime_inputs)?;
    let descriptors = provider.model_descriptors().unwrap_or(&[]).to_vec();
    let catalogue =
        crate::application::catalogue::ResolveCatalogueUseCase.resolve(generation, [descriptors]);
    Ok(CatalogueRuntimeSnapshot {
        provider,
        catalogue,
    })
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ComposeProviderRuntimeUseCase;

impl ComposeProviderRuntimeUseCase {
    pub fn new() -> Self {
        Self
    }

    pub fn compose<C, R, F: ProviderRuntimeFactory<C, R>>(
        &self,
        factory: &F,
        config: &C,
        runtime_inputs: &R,
    ) -> Result<Arc<dyn LlmProvider>, String> {
        factory.compose_runtime(config, runtime_inputs)
    }
}

#[cfg(test)]
#[path = "provider_runtime_tests.rs"]
mod tests;
