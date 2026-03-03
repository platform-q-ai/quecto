/// User message content block builder for the Anthropic API (#188).
///
/// Handles structured content block arrays for user messages containing inline
/// images, model capability filtering, and empty-content skipping.
use crate::domain::message::Message;

/// Returns `true` for models known to support image inputs.
///
/// Defaults to `true` for unknown models (fail-open: send images unless
/// we know the model can't handle them).
pub(super) fn model_supports_vision(model: &str) -> bool {
    // Models known NOT to support vision.
    const NON_VISION: &[&str] = &[
        "claude-instant-1",
        "claude-instant-1.2",
        "claude-2",
        "claude-2.0",
        "claude-2.1",
    ];
    !NON_VISION.iter().any(|&prefix| model.starts_with(prefix))
}

/// Build the Anthropic API content value for a user message.
///
/// Returns:
/// - `Some(String)` — plain text (no images, non-empty)
/// - `Some(Array)` — structured content blocks (text + images)
/// - `None` — message is empty after filtering; caller should skip it
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

    // Add image blocks, filtered by vision capability.
    if supports_vision {
        for img in &m.user_image_blocks {
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
