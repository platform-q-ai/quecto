use super::error::DomainError;

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
