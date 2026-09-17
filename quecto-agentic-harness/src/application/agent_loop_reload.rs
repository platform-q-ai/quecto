//! The loop as the reload-runtime port (#1849): the reload use case swaps
//! the provider and re-applies the persisted tool-policy baseline through
//! it; nothing else writes either after startup.

use std::collections::HashMap;
use std::sync::Arc;

use super::AgentLoopImpl;
use crate::application::catalogue::ports::ReloadRuntime;
use crate::application::providers::ports::LlmProvider;
use crate::domain::tool_descriptor::ProfileAvailabilityScope;

impl ReloadRuntime for AgentLoopImpl {
    fn swap_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        AgentLoopImpl::swap_provider(self, provider);
    }

    fn apply_persisted_tool_policy(
        &mut self,
        entries: &HashMap<String, ProfileAvailabilityScope>,
    ) -> Vec<String> {
        self.apply_persisted_tool_policy_entries(entries)
    }
}
