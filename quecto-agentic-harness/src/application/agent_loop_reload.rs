use crate::application::agent_loop::AgentLoopImpl;
use crate::application::provider_runtime::CatalogueRuntimeSnapshot;
#[cfg(any(test, feature = "test-support"))]
use crate::domain::catalogue::CatalogueSnapshot;
use crate::domain::catalogue::{Availability, ModelRef};
#[cfg(any(test, feature = "test-support"))]
use crate::domain::provider::LlmProvider;
#[cfg(any(test, feature = "test-support"))]
use std::sync::Arc;

impl AgentLoopImpl {
    /// Replace provider and catalogue as one application-owned runtime generation.
    pub fn swap_runtime(&mut self, runtime: CatalogueRuntimeSnapshot) {
        let mut model_max_tokens = None;
        let mut model_context_window = None;

        if let Ok(reference) = ModelRef::parse_qualified(&self.model)
            && let Some(descriptor) = runtime.catalogue.find(&reference)
            && descriptor.availability == Availability::Runnable
        {
            model_max_tokens = descriptor
                .capabilities
                .max_tokens_explicit
                .then_some(descriptor.capabilities.max_tokens);
            model_context_window = descriptor
                .capabilities
                .context_window_explicit
                .then_some(descriptor.capabilities.context_window as usize);
        }

        self.model_max_tokens = model_max_tokens;
        self.model_context_window = model_context_window;
        self.context_manager
            .set_model_context_window(model_context_window);
        self.provider = runtime.provider;
        self.catalogue = runtime.catalogue;
    }

    /// Test compatibility entry point for provider-only swaps.
    #[cfg(any(test, feature = "test-support"))]
    pub fn swap_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        let generation = self.catalogue.generation.saturating_add(1);
        let catalogue = CatalogueSnapshot::new(
            generation,
            provider.model_descriptors().unwrap_or(&[]).to_vec(),
        );
        self.swap_runtime(CatalogueRuntimeSnapshot {
            catalogue,
            provider,
        });
    }
}
