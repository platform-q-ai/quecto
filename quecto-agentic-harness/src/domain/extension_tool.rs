//! Pure vocabulary of an extension tool invocation.
//!
//! A concrete tool implementation creates this request and a transport layer
//! forwards it to the extension client. The reply handle it travels with is
//! the application's (`application::extensions::ports::PendingToolInvocation`,
//! #1960), so this type stays free of runtime channels.

/// A single tool invocation addressed to an extension client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInvocation {
    /// Correlation id echoed by the client in its `tool_result`.
    pub tool_call_id: String,
    /// Name of the tool being invoked.
    pub tool_name: String,
    /// Arguments payload — the LLM's JSON tool-call arguments.
    pub arguments: String,
}
