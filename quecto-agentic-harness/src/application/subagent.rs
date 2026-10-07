// Subagent application logic: context construction.

pub use crate::domain::agents::subagent::{SubagentConfig, validate_agent_id};
use crate::domain::message::Message;

/// The context for a spawned subagent.
#[derive(Debug)]
#[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
pub struct SubagentContext {
    /// The task assigned to this subagent (empty string if none).
    pub task: String,
    /// Conversation history (starts empty — independent from parent).
    pub messages: Vec<Message>,
}

impl SubagentContext {
    /// Create a new subagent context from a spawn config.
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    pub fn from_config(config: &SubagentConfig) -> Self {
        Self {
            task: config.task.clone().unwrap_or_default(),
            messages: vec![], // Independent — no parent history
        }
    }
}

#[cfg(test)]
#[path = "subagent_tests.rs"]
mod tests;
