use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;

use super::error::DomainError;

/// Metadata describing a tool for the LLM.
///
/// Fields use `Cow<'static, str>` so that static tool schemas (the common
/// case — 11 of 12 tools) are zero-cost clones (pointer copy), while
/// dynamic schemas (`ls` with runtime limits) use `Cow::Owned`.
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: Cow<'static, str>,
    pub description: Cow<'static, str>,
    /// JSON Schema string describing the parameters.
    pub parameters_schema: Cow<'static, str>,
}

/// A base64-encoded image block returned by a tool (e.g. `read` on an image file).
#[derive(Debug, Clone)]
pub struct ImageBlock {
    /// MIME type: one of `"image/png"`, `"image/jpeg"`, `"image/gif"`, `"image/webp"`.
    /// Always a static literal — avoids a heap allocation per image block.
    pub mime_type: &'static str,
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

/// A guard that can inspect and block tool calls before execution.
///
/// Guards run in registration order before every `execute()` call on the
/// tool registry.  The first `Err` short-circuits — remaining guards and
/// the tool itself are skipped.  The error string is returned to the LLM
/// as a `ToolResult { is_error: true }` so it can act on the feedback.
pub trait ToolGuard: Send + Sync {
    /// Inspect a tool call before execution.
    /// Return `Ok(())` to allow, `Err(reason)` to block.
    fn check(&self, tool_name: &str, arguments: &str) -> Result<(), String>;
}

/// Port: a registry of tools that can be queried and executed.
///
/// Uses `Pin<Box<dyn Future>>` for dyn-compatibility.
pub trait ToolRegistry: Send + Sync {
    /// Return all tool definitions (for injection into LLM prompts).
    fn definitions(&self) -> &[ToolDefinition];

    /// Return the number of registered tools without cloning definitions.
    ///
    /// Override if `definitions()` is not O(1) (e.g. lazy-building implementations).
    fn tool_count(&self) -> usize {
        self.definitions().len()
    }

    /// Return names of tools registered from extensions (not core tools).
    ///
    /// Default: empty (no extension tracking). Override in registries that
    /// track extension tools separately (e.g. `ToolRegistryImpl`).
    fn extension_names(&self) -> Vec<String> {
        vec![]
    }

    /// Execute a tool by name with JSON arguments.
    fn execute(
        &self,
        name: &str,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>>;
}
