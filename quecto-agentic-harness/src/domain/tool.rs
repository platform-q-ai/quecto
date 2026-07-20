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
///
/// # Error handling contract
///
/// `execute` distinguishes **LLM-addressable** errors from **infrastructure**
/// errors via its return type:
///
/// - `Ok(ToolResult { is_error: true, content })` — the LLM supplied bad
///   input (malformed JSON arguments, missing or invalid field, forbidden
///   path, tool-specific validation failure). The agent loop surfaces
///   `content` back to the LLM so it can read the explanation and retry.
///   **This is the preferred form for anything the LLM can fix.**
///
/// - `Err(DomainError)` — an infrastructure-level failure the LLM cannot
///   reasonably correct (OS I/O error, sandbox violation, timeout, upstream
///   service outage). The agent loop still surfaces these as tool errors,
///   but adapter authors should reserve them for genuinely external failures.
///
/// Both forms terminate the tool call with `is_error: true` from the agent
/// loop's perspective, so neither is "wrong"; the distinction is a style
/// contract for readability and future telemetry, not a functional one.
/// Adapter unit tests should assert the chosen shape to pin down behaviour.
pub trait Tool: Send + Sync {
    /// Return the tool's definition for the LLM.
    fn definition(&self) -> ToolDefinition;

    /// Notify stateful tools that the active session key changed.
    fn set_session_key(&self, _session_key: String) {}

    /// Execute the tool with JSON-encoded arguments.
    ///
    /// See the trait-level docs for the error-handling contract:
    /// prefer `Ok(ToolResult { is_error: true })` for LLM-addressable
    /// errors; use `Err` only for infrastructure failures.
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

    /// Notify stateful tools that the active session key changed.
    fn set_session_key(&self, _session_key: &str) {}

    /// Register a single extension tool.
    ///
    /// Rejects tools that shadow core tool names. Default: no-op.
    fn register_extension(&mut self, _tool: std::sync::Arc<dyn Tool>) {}

    /// Unregister a single extension tool by name.
    ///
    /// No-op if the name is not an extension tool. Default: no-op.
    fn unregister_extension(&mut self, _name: &str) {}

    /// Execute a tool by name with JSON arguments.
    fn execute(
        &self,
        name: &str,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>>;
}

#[cfg(test)]
#[path = "tool_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
