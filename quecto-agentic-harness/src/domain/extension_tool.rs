//! Domain type representing an in-flight extension tool invocation.
//!
//! A concrete tool implementation creates this request and a transport layer
//! forwards it to the extension client, then completes `reply` with the result.

use crate::domain::tool::ToolResult;

/// A single in-flight tool invocation.
pub struct ToolInvocation {
    /// Correlation id echoed by the client in its `tool_result`.
    pub tool_call_id: String,
    /// Name of the tool being invoked.
    pub tool_name: String,
    /// Arguments payload — the LLM's JSON tool-call arguments.
    pub arguments: String,
    /// Deliver the `ToolResult` here.
    pub reply: tokio::sync::oneshot::Sender<ToolResult>,
}
