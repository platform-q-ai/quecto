// Subagent application logic: context construction and workspace inheritance.

use crate::domain::message::Message;
pub use crate::domain::subagent::{SubagentConfig, validate_agent_id};

/// The context for a spawned subagent.
#[derive(Debug)]
pub struct SubagentContext {
    /// The task assigned to this subagent.
    pub task: String,
    /// Conversation history (starts empty — independent from parent).
    pub messages: Vec<Message>,
    /// Whether workspace restriction is inherited from parent.
    pub restrict_to_workspace: bool,
    /// Where to deliver results.
    pub deliver_to: Option<String>,
}

impl SubagentContext {
    /// Create a new subagent context from a spawn config.
    pub fn from_config(config: &SubagentConfig) -> Self {
        Self {
            task: config.task.clone(),
            messages: vec![], // Independent — no parent history
            restrict_to_workspace: config.restrict_to_workspace,
            deliver_to: config.deliver_to.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subagent_context_has_empty_history() {
        let config = SubagentConfig {
            task: "Do stuff".to_string(),
            agent_id: None,
            restrict_to_workspace: true,
            deliver_to: None,
        };
        let ctx = SubagentContext::from_config(&config);
        assert_eq!(ctx.task, "Do stuff");
        assert!(ctx.messages.is_empty());
    }

    #[test]
    fn test_subagent_inherits_restrict_true() {
        let config = SubagentConfig {
            task: "task".to_string(),
            agent_id: None,
            restrict_to_workspace: true,
            deliver_to: None,
        };
        let ctx = SubagentContext::from_config(&config);
        assert!(ctx.restrict_to_workspace);
    }

    #[test]
    fn test_subagent_inherits_restrict_false() {
        let config = SubagentConfig {
            task: "task".to_string(),
            agent_id: None,
            restrict_to_workspace: false,
            deliver_to: None,
        };
        let ctx = SubagentContext::from_config(&config);
        assert!(!ctx.restrict_to_workspace);
    }

    #[test]
    fn test_deliver_to_propagated() {
        let config = SubagentConfig {
            task: "task".to_string(),
            agent_id: None,
            restrict_to_workspace: true,
            deliver_to: Some("telegram:12345".to_string()),
        };
        let ctx = SubagentContext::from_config(&config);
        assert_eq!(ctx.deliver_to.as_deref(), Some("telegram:12345"));
    }
}
