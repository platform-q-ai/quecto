/// User message content block builder for the Anthropic API (#188).
///
/// Handles structured content block arrays for user messages containing inline
/// images, and empty-content skipping. Which images a model is sent is the
/// application's decision (#2421, `domain::conversation::image_input`): every
/// image a message carries here is sent.
use crate::domain::message::Message;

/// Build the Anthropic API content value for a user message.
///
/// Returns:
/// - `Some(String)` — plain text (no images, non-empty)
/// - `Some(Array)` — structured content blocks (text + images)
/// - `None` — message is empty after filtering; **caller must skip it** to
///   avoid sending an empty-content message that the Anthropic API rejects.
///   Note: callers are responsible for ensuring role alternation is maintained
///   when messages are dropped.
pub(super) fn build_user_content(m: &Message) -> Option<serde_json::Value> {
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

    // Add image blocks, filtered by the MIME type allowlist.
    // Anthropic only accepts these four MIME types; others are silently skipped.
    const ALLOWED_MIME: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];
    for img in m
        .user_image_blocks
        .iter()
        .filter(|img| ALLOWED_MIME.contains(&img.mime_type.as_str()))
    {
        blocks.push(serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": img.mime_type,
                "data": img.data,
            }
        }));
    }

    if blocks.is_empty() {
        None // All content filtered — skip this message
    } else if blocks.len() == 1 && blocks[0]["type"] == "text" {
        // Only text remains after MIME filtering — use plain string, as a
        // message without images is sent.
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
