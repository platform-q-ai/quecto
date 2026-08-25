use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::catalogue::{Availability, ModelRef};
use crate::domain::provider::LlmProvider;
use std::sync::Arc;

impl AgentLoopImpl {
    /// Replace the LLM provider after config reload and re-derive limits for the active model.
    pub fn swap_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        if let Ok(reference) = ModelRef::parse_qualified(&self.model)
            && let Some(descriptor) = provider
                .model_descriptors()
                .and_then(|models| models.iter().find(|m| m.reference == reference))
        {
            if descriptor.availability == Availability::Runnable {
                self.model_max_tokens = descriptor
                    .capabilities
                    .max_tokens_explicit
                    .then_some(descriptor.capabilities.max_tokens);
                self.model_context_window = descriptor
                    .capabilities
                    .context_window_explicit
                    .then_some(descriptor.capabilities.context_window as usize);
                self.context_manager
                    .set_model_context_window(self.model_context_window);
            } else {
                self.model_max_tokens = None;
                self.model_context_window = None;
                self.context_manager.set_model_context_window(None);
            }
        }
        self.provider = provider;
    }
}
