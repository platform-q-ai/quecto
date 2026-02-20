use std::future::Future;
use std::pin::Pin;

use super::{
    error::DomainError,
    message::{LlmResponse, Message},
    tool::ToolDefinition,
};

/// Parameters for a chat request to an LLM provider.
#[derive(Debug, Clone)]
pub struct ChatRequest<'a> {
    pub messages: &'a [Message],
    pub tools: &'a [ToolDefinition],
    pub model: &'a str,
    pub max_tokens: u32,
    pub temperature: f32,
}

/// Port: an LLM provider that can process chat requests.
pub trait LlmProvider: Send + Sync + std::fmt::Debug {
    /// Human-readable provider name (e.g. "openai", "anthropic").
    fn name(&self) -> &str;

    /// Send a chat request and return the LLM response.
    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>>;
}
