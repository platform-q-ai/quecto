//! Application-level provider runtime composition.
//!
//! The application owns the composition use-case seam. Concrete configuration
//! shapes stay outside this layer: callers select a config type via the generic
//! factory implementation, keeping application independent of infrastructure.

use std::sync::Arc;

use crate::domain::provider::LlmProvider;

pub trait ProviderRuntimeFactory<C> {
    fn compose_runtime(
        &self,
        config: &C,
        base_dir: &std::path::Path,
        http_client: &reqwest::Client,
    ) -> Result<Arc<dyn LlmProvider>, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ComposeProviderRuntimeUseCase;

impl ComposeProviderRuntimeUseCase {
    pub fn new() -> Self {
        Self
    }

    pub fn compose<C, F: ProviderRuntimeFactory<C>>(
        &self,
        factory: &F,
        config: &C,
        base_dir: &std::path::Path,
        http_client: &reqwest::Client,
    ) -> Result<Arc<dyn LlmProvider>, String> {
        factory.compose_runtime(config, base_dir, http_client)
    }
}

#[cfg(test)]
#[path = "provider_runtime_tests.rs"]
mod tests;
