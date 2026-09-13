//! Capability-local ports of the extensions capability (#1960).
//!
//! An extension is the composition unit that contributes tools and optional
//! system-prompt context to the agent. Native (compiled-in) extensions and
//! UDS-registered extensions both implement [`Extension`].
use std::sync::Arc;

use crate::application::tools::ports::Tool;
use crate::domain::extension_tool::ToolInvocation;
use crate::domain::tool::ToolResult;

/// The handle a forwarded invocation's result is delivered on.
pub type ToolReply = tokio::sync::oneshot::Sender<ToolResult>;

/// An in-flight extension tool invocation: the pure request together with
/// the handle the transport completes with its result (#1960).
pub struct PendingToolInvocation {
    pub invocation: ToolInvocation,
    /// Deliver the `ToolResult` here.
    pub reply: ToolReply,
}

/// An extension contributes tools and optional system prompt context.
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
#[path = "ports_tests.rs"]
mod tests;
