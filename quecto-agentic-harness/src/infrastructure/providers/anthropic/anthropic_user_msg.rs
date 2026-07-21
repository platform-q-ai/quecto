/// User message content block builder for the Anthropic API (#188).
///
/// Handles structured content block arrays for user messages containing inline
/// images, model capability filtering, and empty-content skipping.
use crate::domain::message::Message;

/// Returns `true` for models known to support image inputs.
///
/// Uses an allow-list approach (fail-closed): unknown models are assumed
/// to NOT support vision. This avoids sending images to models that would
/// reject them. When a new vision model family is released, add its
/// lowercase prefix here.
///
/// Matching is case-insensitive for consistency with `domain::message::model_pricing`.
/// If vision detection is ever needed for other providers, consider migrating
/// this function to `domain::message` alongside `model_pricing`.
pub(super) fn model_supports_vision(model: &str) -> bool {
    // Lowercase prefixes for model families known to support vision (Claude 3+).
    // All Claude 3.x IDs use dashes (e.g. `claude-3-opus-…`, `claude-3-5-sonnet-…`).
    const VISION_PREFIXES: &[&str] = &[
        "claude-3-",
        "claude-sonnet-",
        "claude-opus-",
        "claude-haiku-",
    ];
    let model_lower = model.to_lowercase();
    VISION_PREFIXES
        .iter()
        .any(|prefix| model_lower.starts_with(prefix))
}

/// Build the Anthropic API content value for a user message.
///
/// Returns:
/// - `Some(String)` — plain text (no images, non-empty)
/// - `Some(Array)` — structured content blocks (text + images)
/// - `None` — message is empty after filtering; **caller must skip it** to
///   avoid sending an empty-content message that the Anthropic API rejects.
///   Note: callers are responsible for ensuring role alternation is maintained
///   when messages are dropped.
pub(super) fn build_user_content(m: &Message, supports_vision: bool) -> Option<serde_json::Value> {
    let has_images = !m.user_image_blocks.is_empty();

    if !has_images {
        // Fast path: plain string, skip whitespace-only messages.
        let text = m.content.trim();
        if text.is_empty() {
            return None;
        }
        return Some(serde_json::Value::String(text.to_string()));
    }

    // Build structured content block array.
    let mut blocks: Vec<serde_json::Value> = Vec::new();

    // Add text block first (if non-empty).
    let text = m.content.trim();
    if !text.is_empty() {
        blocks.push(serde_json::json!({"type": "text", "text": text}));
    }

    // Add image blocks, filtered by vision capability and MIME type allowlist.
    // Anthropic only accepts these four MIME types; others are silently skipped.
    const ALLOWED_MIME: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];
    if supports_vision {
        for img in &m.user_image_blocks {
            if !ALLOWED_MIME.contains(&img.mime_type.as_str()) {
                continue;
            }
            blocks.push(serde_json::json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": img.mime_type,
                    "data": img.data,
                }
            }));
        }
    }

    if blocks.is_empty() {
        None // All content filtered — skip this message
    } else if blocks.len() == 1 && blocks[0]["type"] == "text" {
        // Only text remains after image filtering — use plain string for
        // backward compatibility with non-vision model paths.
        blocks[0]["text"]
            .as_str()
            .map(|t| serde_json::Value::String(t.to_string()))
    } else {
        Some(serde_json::Value::Array(blocks))
    }
}

#[cfg(test)]
#[path = "anthropic_user_msg_tests.rs"]
mod tests;
