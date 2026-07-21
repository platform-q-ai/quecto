//! Extension trait: a modular unit that contributes tools and optional
//! system prompt context to the agent.

use std::sync::Arc;

use super::tool::Tool;

/// An extension contributes tools and optional system prompt context.
///
/// Extensions are the composition unit for adding capabilities to the agent.
/// Native (compiled-in) extensions and UDS-registered extensions both implement
/// this trait.
pub trait Extension: Send + Sync {
    /// Unique name for this extension.
    fn name(&self) -> &str;

    /// Human-readable description of this extension.
    fn description(&self) -> &str {
        ""
    }

    /// Tools this extension provides.
    fn tools(&self) -> Vec<Arc<dyn Tool>>;

    /// Optional text injected into the system prompt each turn.
    /// Called fresh each time so stateful extensions can reflect current state.
    fn system_prompt_snippet(&self) -> Option<String> {
        None
    }
}

#[cfg(test)]
#[path = "extension_tests.rs"]
mod tests;
