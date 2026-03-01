use std::sync::Arc;

use super::error::DomainError;

/// A live progress event emitted by the agent loop during processing.
///
/// Used by the REPL progress renderer to display tool activity and a spinner
/// to the user while the agent is thinking or executing tools. Events are
/// delivered via a [`ProgressCallback`] registered in [`AgentLoopConfig`].
#[derive(Debug, Clone)]
pub enum AgentProgressEvent {
    /// The agent is waiting on the LLM for a response (thinking).
    Thinking,
    /// A tool call has been dispatched.
    ToolStarted {
        /// The name of the tool being called.
        name: String,
        /// A short preview of the tool arguments (truncated).
        input_preview: String,
    },
    /// A tool call has completed.
    ToolFinished {
        /// The name of the tool that finished.
        name: String,
        /// How long the tool took to execute in milliseconds.
        duration_ms: u64,
        /// Whether the tool returned an error.
        is_error: bool,
    },
    /// The agent loop has produced a final text response and is done.
    Done,
}

/// A synchronous, non-blocking callback that receives live agent progress events.
///
/// The callback must be `Send + Sync` because it is called from an async context.
/// It must not block — use a channel send or mutex push, never I/O.
pub type ProgressCallback = Arc<dyn Fn(AgentProgressEvent) + Send + Sync>;

/// Information about a configured agent at startup.
#[derive(Debug, Clone)]
pub struct AgentInfo {
    /// Number of tools loaded in the registry.
    pub tool_count: usize,
    /// Number of skills loaded.
    pub skill_count: usize,
}

/// The result of an agent loop processing run.
#[derive(Debug, Clone)]
pub struct AgentResult {
    /// The final assistant text response.
    pub response: String,
    /// Number of tool iterations performed.
    pub tool_iterations: u32,
    /// Whether the iteration limit was reached.
    pub iteration_limit_reached: bool,
}

impl AgentResult {
    pub fn text(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
            tool_iterations: 0,
            iteration_limit_reached: false,
        }
    }
}

/// Port: the agent loop that processes messages through LLM + tools.
pub trait AgentLoop: Send + Sync {
    /// Process a conversation: send messages to the LLM, execute tool calls,
    /// and return the final assistant response with metadata.
    fn process<'a>(
        &'a self,
        messages: &'a mut Vec<super::message::Message>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<AgentResult, DomainError>> + Send + 'a>,
    >;

    /// Return information about this agent's configuration.
    fn info(&self) -> AgentInfo;
}
