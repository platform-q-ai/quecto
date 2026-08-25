//! Application-level provider runtime composition.
//!
//! The application owns the composition use-case seam. Concrete configuration
//! and runtime input shapes stay outside this layer: callers select those types
//! via the generic factory implementation, keeping application independent of
//! infrastructure.

use std::sync::Arc;

use crate::application::catalogue::{CatalogueSource, ResolveCatalogueUseCase};
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

/// Compose provider routing and its catalogue from exactly the same adapter
/// result. `base_layers` are the lower-precedence catalogue sources (built-in
/// metadata, then user-owned configuration); the runtime layer composed here
/// always has the highest precedence because it alone carries credential- and
/// adapter-derived availability. Both halves of the returned snapshot describe
/// the same generation.
pub fn compose_catalogue_runtime<C, R, F: ProviderRuntimeFactory<C, R>>(
    factory: &F,
    config: &C,
    runtime_inputs: &R,
    generation: u64,
    base_layers: &[&dyn CatalogueSource],
) -> Result<CatalogueRuntimeSnapshot, String> {
    let provider = ComposeProviderRuntimeUseCase::new().compose(factory, config, runtime_inputs)?;
    let runtime_layer = RuntimeDescriptorSource(provider.model_descriptors().unwrap_or(&[]));
    let mut sources: Vec<&dyn CatalogueSource> = base_layers.to_vec();
    sources.push(&runtime_layer);
    let resolved = ResolveCatalogueUseCase.resolve_sources(generation, &sources);
    for skipped in &resolved.skipped {
        tracing::warn!(
            source = %skipped.source,
            error = %skipped.error,
            "catalogue source skipped; resolving remaining layers"
        );
    }
    Ok(CatalogueRuntimeSnapshot {
        provider,
        catalogue: resolved.snapshot,
    })
}

/// The composed provider runtime as the highest-precedence catalogue layer.
struct RuntimeDescriptorSource<'a>(&'a [crate::domain::catalogue::ModelDescriptor]);

impl CatalogueSource for RuntimeDescriptorSource<'_> {
    fn id(&self) -> &str {
        "runtime"
    }

    fn load(&self) -> Result<Vec<crate::domain::catalogue::ModelDescriptor>, String> {
        Ok(self.0.to_vec())
    }
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
