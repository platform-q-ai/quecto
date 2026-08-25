//! Application-owned provider composition boundary.

use std::sync::Arc;

use crate::application::catalogue_runtime::CatalogueRuntimeSnapshot;
use crate::domain::catalogue::CatalogueSnapshot;
use crate::domain::provider::LlmProvider;

pub trait ProviderRuntimePort<C, R> {
    fn compose(&self, config: &C, inputs: &R) -> Result<Arc<dyn LlmProvider>, String>;
}

/// Owns both startup composition and reload publication shape.
pub struct ProviderRuntimeApplication<P> {
    port: P,
}

impl<P> ProviderRuntimeApplication<P> {
    pub fn new(port: P) -> Self {
        Self { port }
    }

    pub fn compose<C, R>(&self, config: &C, inputs: &R) -> Result<Arc<dyn LlmProvider>, String>
    where
        P: ProviderRuntimePort<C, R>,
    {
        self.port.compose(config, inputs)
    }

    pub fn compose_reload<C, R>(
        &self,
        config: &C,
        inputs: &R,
    ) -> Result<CatalogueRuntimeSnapshot, String>
    where
        P: ProviderRuntimePort<C, R>,
    {
        let provider = self.port.compose(config, inputs)?;
        let descriptors = provider.model_descriptors().unwrap_or(&[]).to_vec();
        Ok(CatalogueRuntimeSnapshot {
            provider,
            catalogue: CatalogueSnapshot::new(0, descriptors),
        })
    }
}

#[cfg(test)]
#[path = "provider_runtime_tests.rs"]
mod tests;
