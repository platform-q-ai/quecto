//! The session state a reload acts on (#1849): the provider the agent loop
//! routes every request through and the persisted tool-policy baseline it
//! resolves tool availability against. The loop implements it; the reload
//! use case is the only writer.

use std::collections::HashMap;
use std::sync::Arc;

use crate::application::providers::ports::LlmProvider;
use crate::domain::tool_descriptor::ProfileAvailabilityScope;

pub trait ReloadRuntime {
    /// Route every subsequent request through `provider`.
    fn swap_provider(&mut self, provider: Arc<dyn LlmProvider>);
    /// Replace the persisted tool-policy baseline with `entries` and clear
    /// the live-only overlays layered over the previous one. Returns the
    /// stable ids that matched no registered tool.
    fn apply_persisted_tool_policy(
        &mut self,
        entries: &HashMap<String, ProfileAvailabilityScope>,
    ) -> Vec<String>;
}
