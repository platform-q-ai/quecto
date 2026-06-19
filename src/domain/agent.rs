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
    Thinking {
        /// Estimated tokens currently in the conversation context.
        context_tokens: usize,
        /// Configured maximum context token budget.
        max_context_tokens: usize,
        /// Provider name serving the request (e.g. "openai").
        provider: String,
        /// Model name serving the request (e.g. "gpt-5.5").
        model: String,
    },
    /// A tool call has been dispatched.
    ToolStarted {
        /// The provider-assigned tool call ID (e.g. `call_abc123`).
        /// Used by UDS clients to correlate start → end event pairs.
        tool_call_id: String,
        /// The name of the tool being called.
        name: String,
        /// Raw tool arguments JSON (not pre-truncated).
        ///
        /// Consumers (e.g. `ProgressRenderer`) are responsible for any
        /// display-width truncation. Keeping the raw value here avoids baking
        /// a terminal-width concern into the application layer.
        arguments: String,
    },
    /// A tool call has completed.
    ToolFinished {
        /// The provider-assigned tool call ID (e.g. `call_abc123`).
        /// Used by UDS clients to correlate start → end event pairs.
        tool_call_id: String,
        /// The name of the tool that finished.
        name: String,
        /// Raw tool arguments JSON (not pre-truncated).
        arguments: String,
        /// The tool's result content (text output).
        result_content: String,
        /// How long the tool took to execute in milliseconds.
        duration_ms: u64,
        /// Whether the tool returned an error.
        is_error: bool,
    },
    /// An incremental text token arrived from the LLM during streaming.
    Token(String),
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
    /// Latest prompt/provider input token count for this run.
    pub input_tokens: u32,
    /// Estimated tokens currently occupying the active, pruned context after this run.
    pub context_tokens: usize,
    /// Cumulative completion tokens from all LLM calls in this run.
    pub output_tokens: u32,
    /// Cumulative billed input tokens from all LLM calls in this run.
    pub billed_input_tokens: u64,
    /// Cumulative billed output tokens from all LLM calls in this run.
    pub billed_output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Cumulative provider-reported cost for this run, in micro-USD.
    pub cost_micro_usd: u64,
}

impl AgentResult {
    pub fn text(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
            tool_iterations: 0,
            iteration_limit_reached: false,
            input_tokens: 0,
            context_tokens: 0,
            output_tokens: 0,
            billed_input_tokens: 0,
            billed_output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_micro_usd: 0,
        }
    }

    pub fn turn_tokens(&self) -> u32 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    pub fn has_usage(&self) -> bool {
        self.turn_tokens() > 0
            || self.billed_input_tokens > 0
            || self.billed_output_tokens > 0
            || self.cache_read_tokens > 0
            || self.cache_write_tokens > 0
            || self.cost_micro_usd > 0
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
