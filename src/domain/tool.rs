use std::future::Future;
use std::pin::Pin;

use super::error::DomainError;

/// Metadata describing a tool for the LLM.
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema string describing the parameters.
    pub parameters_schema: String,
}

/// A base64-encoded image block returned by a tool (e.g. `read` on an image file).
#[derive(Debug, Clone)]
pub struct ImageBlock {
    /// MIME type: "image/png", "image/jpeg", "image/gif", or "image/webp".
    pub mime_type: String,
    /// Base64-encoded image bytes (standard encoding, no line breaks).
    pub data: String,
}

/// The result of executing a tool.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    /// Optional image blocks (e.g. when `read` is called on an image file).
    /// Empty for all non-image tools — zero-cost default.
    pub image_blocks: Vec<ImageBlock>,
}

/// Port: a tool the agent can invoke.
///
/// Uses `Pin<Box<dyn Future>>` return type for dyn-compatibility,
/// allowing tools to be stored in a registry as `Arc<dyn Tool>`.
pub trait Tool: Send + Sync {
    /// Return the tool's definition for the LLM.
    fn definition(&self) -> ToolDefinition;

    /// Execute the tool with JSON-encoded arguments.
    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>>;
}

/// Port: a registry of tools that can be queried and executed.
///
/// Uses `Pin<Box<dyn Future>>` for dyn-compatibility.
pub trait ToolRegistry: Send + Sync {
    /// Return all tool definitions (for injection into LLM prompts).
    fn definitions(&self) -> Vec<ToolDefinition>;

    /// Execute a tool by name with JSON arguments.
    fn execute(
        &self,
        name: &str,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>>;
}
