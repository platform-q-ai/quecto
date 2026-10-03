//! What an MCP `tools/call` result becomes as a quecto `tool_result` (#2423).
//!
//! Text items are joined with newlines, as before. Each `image` item is
//! admitted by `quecto_image`, by the same strict rules the agent applies
//! (MIME allowlist, strict base64, at most 3.75 MiB decoded, the type's
//! signature, a readable header), and becomes an `imageBlocks` entry. An
//! image that cannot be sent is a text marker in its place,
//! `[image omitted: <reason>]`, so the model knows it was there. At most
//! [`quecto_image::MAX_IMAGES_PER_MESSAGE`] images are sent, and only as
//! many as fit one UDS line with the text. A result with neither text nor
//! images (only resources, say) is its JSON, as before.
use quecto_image::{ImageAttachment, ImagePayload, MAX_IMAGES_PER_MESSAGE};
use quecto_line_io::PROTOCOL_LINE_CAP_BYTES;
use serde_json::Value;

/// The marker for an `image` item without string `mimeType` and `data`.
pub const MALFORMED_IMAGE_MARKER: &str =
    "[image omitted: an image item needs string \"mimeType\" and \"data\"]";
/// The marker for each image past [`MAX_IMAGES_PER_MESSAGE`].
pub const TOO_MANY_IMAGES_MARKER: &str = "[image omitted: more than 8 images in one result]";
/// The marker for an image the UDS line has no room for.
pub const LINE_LIMIT_MARKER: &str =
    "[image omitted: the result would pass the Quecto UDS line limit]";

// The too-many marker spells the limit out.
const _: () = assert!(MAX_IMAGES_PER_MESSAGE == 8);

/// An MCP tool result as quecto's `tool_result` carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolResult {
    /// The text the model reads.
    pub content: String,
    /// The images, each admitted by `quecto_image`.
    pub image_blocks: Vec<ImageAttachment>,
}

impl McpToolResult {
    /// A result of `content` alone.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            image_blocks: Vec::new(),
        }
    }
}

/// The `tool_result` an MCP `result` becomes.
pub fn mcp_tool_result(result: &Value) -> McpToolResult {
    let items = result
        .get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut image_blocks = Vec::new();
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("image") => match admit(item, image_blocks.len()) {
                Ok(image) => image_blocks.push(image),
                Err(marker) => lines.push(marker),
            },
            // Every other item keeps its handling: its text, if it has any.
            _ => lines.extend(item.get("text").and_then(Value::as_str).map(str::to_owned)),
        }
    }
    let content = lines.join("\n");
    let content = match (content.is_empty(), image_blocks.is_empty()) {
        (true, true) => serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string()),
        (true, false) | (false, _) => content,
    };
    fit_to_line(McpToolResult {
        content,
        image_blocks,
    })
}

/// The image `item` carries, admitted, while fewer than the limit are;
/// else the marker in its place.
fn admit(item: &Value, admitted: usize) -> Result<ImageAttachment, String> {
    let field = |name| item.get(name).and_then(Value::as_str);
    let (Some(mime_type), Some(data)) = (field("mimeType"), field("data")) else {
        return Err(MALFORMED_IMAGE_MARKER.to_owned());
    };
    match admitted < MAX_IMAGES_PER_MESSAGE {
        true => ImageAttachment::new(ImagePayload::new(mime_type, data))
            .map_err(|refusal| format!("[image omitted: {refusal}]")),
        false => Err(TOO_MANY_IMAGES_MARKER.to_owned()),
    }
}

/// `result` with its last images replaced by markers until its line fits.
fn fit_to_line(mut result: McpToolResult) -> McpToolResult {
    while !fits(&render(LONGEST_CALL_ID, &result, false)) {
        let Some(_) = result.image_blocks.pop() else {
            break;
        };
        match result.content.is_empty() {
            true => {}
            false => result.content.push('\n'),
        }
        result.content.push_str(LINE_LIMIT_MARKER);
    }
    result
}

/// A call id as long as one is budgeted for: quecto's are `uds-<uuid>`.
const LONGEST_CALL_ID: &str =
    "uds-0000000000000000000000000000000000000000000000000000000000000000";

/// Whether `line` and its newline fit the UDS line limit.
fn fits(line: &str) -> bool {
    line.len() < PROTOCOL_LINE_CAP_BYTES
}

/// The `tool_result` line for `result`, without its newline: always one
/// the agent reads. A result whose line would pass the UDS line limit (its
/// text alone is too long) is an error result saying so.
pub fn tool_result_line(tool_call_id: &str, result: &McpToolResult, is_error: bool) -> String {
    let line = render(tool_call_id, result, is_error);
    match fits(&line) {
        true => line,
        false => {
            let too_large = McpToolResult::text(format!(
                "MCP result too large for the Quecto UDS line limit: {} bytes; the limit is {PROTOCOL_LINE_CAP_BYTES}",
                line.len() + 1
            ));
            let line = render(tool_call_id, &too_large, true);
            assert!(fits(&line), "the too-large result fits the line");
            line
        }
    }
}

/// The `tool_result` JSON for `result`: `imageBlocks` only when it has
/// images, so a text result's line is as before.
fn render(tool_call_id: &str, result: &McpToolResult, is_error: bool) -> String {
    let mut value = serde_json::json!({
        "type": "tool_result",
        "toolCallId": tool_call_id,
        "content": result.content,
        "isError": is_error,
    });
    match result.image_blocks.is_empty() {
        true => {}
        false => value["imageBlocks"] = serde_json::json!(result.image_blocks),
    }
    value.to_string()
}

#[cfg(test)]
#[path = "content_tests.rs"]
mod tests;
