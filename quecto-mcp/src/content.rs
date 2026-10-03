//! What an MCP `tools/call` result becomes as a quecto `tool_result` (#2423).
use quecto_image::ImageAttachment;
use serde_json::Value;

/// An MCP tool result as quecto's `tool_result` carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolResult {
    /// The text the model reads.
    pub content: String,
    /// The images, each admitted by `quecto_image`.
    pub image_blocks: Vec<ImageAttachment>,
}

/// The marker for an `image` item without string `mimeType` and `data`.
pub const MALFORMED_IMAGE_MARKER: &str =
    "[image omitted: an image item needs string \"mimeType\" and \"data\"]";
/// The marker for each image past [`quecto_image::MAX_IMAGES_PER_MESSAGE`].
pub const TOO_MANY_IMAGES_MARKER: &str = "[image omitted: more than 8 images in one result]";
/// The marker for an image the UDS line has no room for.
pub const LINE_LIMIT_MARKER: &str =
    "[image omitted: the result would pass the Quecto UDS line limit]";

/// The `tool_result` an MCP `result` becomes.
pub fn mcp_tool_result(result: &Value) -> McpToolResult {
    McpToolResult {
        content: super::format_mcp_result(result),
        image_blocks: Vec::new(),
    }
}

/// The `tool_result` line for `result`.
pub fn tool_result_line(tool_call_id: &str, result: &McpToolResult, is_error: bool) -> String {
    serde_json::json!({"type": "tool_result", "toolCallId": tool_call_id, "content": result.content, "isError": is_error}).to_string()
}

#[cfg(test)]
#[path = "content_tests.rs"]
mod tests;
