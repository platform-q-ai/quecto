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
    /// Whether the tool failed.
    pub is_error: bool,
}

impl McpToolResult {
    /// A result of `content` alone.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            image_blocks: Vec::new(),
            is_error: false,
        }
    }

    /// An error result of `content` alone.
    pub fn error(content: impl Into<String>) -> Self {
        Self {
            is_error: true,
            ..Self::text(content)
        }
    }
}

/// The `tool_result` an MCP `result` becomes. Its strings are moved out,
/// not copied: an image's base64 is megabytes.
pub fn mcp_tool_result(mut result: Value) -> McpToolResult {
    let is_error = matches!(result.get("isError"), Some(Value::Bool(true)));
    let mut items = match result.get_mut("content") {
        Some(Value::Array(items)) => std::mem::take(items),
        _ => Vec::new(),
    };
    let mut lines: Vec<String> = Vec::new();
    let mut image_blocks = Vec::new();
    for item in &mut items {
        match item.get("type").and_then(Value::as_str) {
            Some("image") => match admit(item, image_blocks.len()) {
                Ok(image) => image_blocks.push(image),
                Err(marker) => lines.push(marker),
            },
            // Every other item keeps its handling: its text, if it has any.
            _ => lines.extend(take_str(item, "text")),
        }
    }
    let content = lines.join("\n");
    let content = match (content.is_empty(), image_blocks.is_empty()) {
        // No image item was read and every text taken was empty, so the
        // items are as they came: the result is its JSON, as before.
        (true, true) => {
            result["content"] = Value::Array(items);
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
        }
        (true, false) | (false, _) => content,
    };
    fit_to_line(McpToolResult {
        content,
        image_blocks,
        is_error,
    })
}

/// The string at `key` of `item`, moved out.
fn take_str(item: &mut Value, key: &str) -> Option<String> {
    match item.get_mut(key)? {
        Value::String(text) => Some(std::mem::take(text)),
        _ => None,
    }
}

/// The image `item` carries, admitted, while fewer than the limit are;
/// else the marker in its place.
fn admit(item: &mut Value, admitted: usize) -> Result<ImageAttachment, String> {
    let (Some(mime_type), Some(data)) = (take_str(item, "mimeType"), take_str(item, "data")) else {
        return Err(MALFORMED_IMAGE_MARKER.to_owned());
    };
    match admitted < MAX_IMAGES_PER_MESSAGE {
        true => ImageAttachment::new(ImagePayload::new(mime_type, data))
            .map_err(|refusal| format!("[image omitted: {refusal}]")),
        false => Err(TOO_MANY_IMAGES_MARKER.to_owned()),
    }
}

/// `result` with its last images replaced by markers until its line fits,
/// measured without rendering it.
fn fit_to_line(mut result: McpToolResult) -> McpToolResult {
    let mut content_len = json_str_len(&result.content);
    // `isError` counted as `false`, the longer.
    while line_len(LONGEST_CALL_ID, (content_len, false), &result.image_blocks)
        >= PROTOCOL_LINE_CAP_BYTES
    {
        let Some(_) = result.image_blocks.pop() else {
            break;
        };
        match result.content.is_empty() {
            true => {}
            false => {
                result.content.push('\n');
                content_len += 2;
            }
        }
        result.content.push_str(LINE_LIMIT_MARKER);
        content_len += LINE_LIMIT_MARKER.len();
    }
    result
}

/// A call id as long as one is budgeted for: quecto's are `uds-<uuid>`.
const LONGEST_CALL_ID: &str =
    "uds-0000000000000000000000000000000000000000000000000000000000000000";

/// The `tool_result` line for `result`, without its newline: always one
/// the agent reads, rendered once. A result whose line would pass the UDS
/// line limit (its text alone is too long) is an error result saying so.
pub fn tool_result_line(tool_call_id: &str, result: &McpToolResult) -> String {
    let content_len = json_str_len(&result.content);
    let len = line_len(
        tool_call_id,
        (content_len, result.is_error),
        &result.image_blocks,
    );
    let (line, expected) = match len < PROTOCOL_LINE_CAP_BYTES {
        true => (render(tool_call_id, result), len),
        false => {
            let too_large = McpToolResult::error(format!(
                "MCP result too large for the Quecto UDS line limit: {} bytes; the limit is {PROTOCOL_LINE_CAP_BYTES}",
                len + 1
            ));
            let expected = line_len(tool_call_id, (json_str_len(&too_large.content), true), &[]);
            (render(tool_call_id, &too_large), expected)
        }
    };
    assert!(line.len() < PROTOCOL_LINE_CAP_BYTES, "the line fits");
    debug_assert_eq!(line.len(), expected, "the measure is the rendering's");
    line
}

/// A `tool_result` line's fields, in the order they are written.
#[derive(serde::Serialize)]
struct Line<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(rename = "toolCallId")]
    tool_call_id: &'a str,
    content: &'a str,
    #[serde(rename = "isError")]
    is_error: bool,
    /// Only when there are images, so a text result's line is as before.
    #[serde(rename = "imageBlocks", skip_serializing_if = "<[_]>::is_empty")]
    image_blocks: &'a [ImageAttachment],
}

fn render(tool_call_id: &str, result: &McpToolResult) -> String {
    serde_json::to_string(&Line {
        kind: "tool_result",
        tool_call_id,
        content: &result.content,
        is_error: result.is_error,
        image_blocks: &result.image_blocks,
    })
    .expect("a tool_result line serializes")
}

/// The bytes of a line with `content_len` bytes of JSON content and
/// `is_error`, as [`render`] writes it.
fn line_len(
    tool_call_id: &str,
    (content_len, is_error): (usize, bool),
    images: &[ImageAttachment],
) -> usize {
    let images_len = match images.len() {
        0 => 0,
        count => {
            r#","imageBlocks":[]"#.len()
                + (count - 1)
                + images
                    .iter()
                    .map(|image| {
                        r#"{"mimeType":,"data":}"#.len()
                            + json_str_len(image.mime_type())
                            + json_str_len(image.data())
                    })
                    .sum::<usize>()
        }
    };
    r#"{"type":"tool_result","toolCallId":,"content":,"isError":false}"#.len()
        + json_str_len(tool_call_id)
        + content_len
        + images_len
        - usize::from(is_error)
}

/// The bytes `text` takes as a JSON string, quotes and escapes included,
/// as `serde_json` writes it.
fn json_str_len(text: &str) -> usize {
    2 + text
        .bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0c => 2,
            0x00..=0x1f => 6,
            _ => 1,
        })
        .sum::<usize>()
}

#[cfg(test)]
#[path = "content_tests.rs"]
mod tests;
