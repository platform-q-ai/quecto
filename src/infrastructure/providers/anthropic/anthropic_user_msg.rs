/// User message content block builder for the Anthropic API (#188).
///
/// Handles structured content block arrays for user messages containing inline
/// images, model capability filtering, and empty-content skipping.
use crate::domain::message::Message;

/// Returns `true` for models known to support image inputs.
///
/// Uses an allow-list approach (fail-closed): unknown models are assumed
/// to NOT support vision. This avoids sending images to models that would
/// reject them. When a new vision model is released, add its prefix here.
pub(super) fn model_supports_vision(model: &str) -> bool {
    // Prefixes for model families known to support vision (Claude 3+).
    const VISION_PREFIXES: &[&str] = &[
        "claude-3-",
        "claude-3.",
        "claude-sonnet-",
        "claude-opus-",
        "claude-haiku-",
    ];
    VISION_PREFIXES
        .iter()
        .any(|prefix| model.starts_with(prefix))
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
mod tests {
    use super::*;

    // --- #310: Vision allow-list (fail-closed) ---

    #[test]
    fn known_vision_models_return_true() {
        // All Claude 3+ models support vision
        assert!(model_supports_vision("claude-3-opus-20240229"));
        assert!(model_supports_vision("claude-3-sonnet-20240229"));
        assert!(model_supports_vision("claude-3-haiku-20240307"));
        assert!(model_supports_vision("claude-3-5-sonnet-20241022"));
        assert!(model_supports_vision("claude-sonnet-4-20250514"));
        assert!(model_supports_vision("claude-opus-4-5"));
    }

    #[test]
    fn known_non_vision_models_return_false() {
        assert!(!model_supports_vision("claude-instant-1"));
        assert!(!model_supports_vision("claude-instant-1.2"));
        assert!(!model_supports_vision("claude-2"));
        assert!(!model_supports_vision("claude-2.0"));
        assert!(!model_supports_vision("claude-2.1"));
    }

    #[test]
    fn unknown_model_returns_false_fail_closed() {
        // #310: Unknown models should NOT be assumed to support vision
        assert!(!model_supports_vision("unknown-future-model"));
        assert!(!model_supports_vision("gpt-4o"));
        assert!(!model_supports_vision("some-random-model"));
    }
}
