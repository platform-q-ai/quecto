//! Extension trait: a modular unit that contributes tools and optional
//! system prompt context to the agent.

use std::sync::Arc;

use super::tool::Tool;

/// An extension contributes tools and optional system prompt context.
///
/// Extensions are the composition unit for adding capabilities to the agent.
/// Compiled-in tools (bash, read, etc.) and script-based tools from disk
/// both implement this trait.
pub trait Extension: Send + Sync {
    /// Unique name for this extension.
    fn name(&self) -> &str;

    /// Tools this extension provides.
    fn tools(&self) -> Vec<Arc<dyn Tool>>;

    /// Optional text injected into the system prompt each turn.
    /// Called fresh each time so stateful extensions can reflect current state.
    fn system_prompt_snippet(&self) -> Option<String> {
        None
    }

    /// Whether this extension was loaded from a script on disk.
    /// Used by the registry to distinguish script extensions (hot-reloadable)
    /// from compiled-in builtins during reload.
    fn is_script(&self) -> bool {
        false
    }
}
