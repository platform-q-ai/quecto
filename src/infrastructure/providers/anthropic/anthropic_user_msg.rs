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

    #[test]
    fn matching_is_case_insensitive() {
        // Consistent with domain::message::model_pricing case handling
        assert!(model_supports_vision("Claude-3-Opus-20240229"));
        assert!(model_supports_vision("CLAUDE-SONNET-4-20250514"));
    }

    // --- build_user_content ---

    #[test]
    fn plain_text_returns_string() {
        let m = Message::user("hello world");
        let content = build_user_content(&m, false);
        assert_eq!(
            content,
            Some(serde_json::Value::String("hello world".to_string()))
        );
    }

    #[test]
    fn empty_content_returns_none() {
        let m = Message::user("");
        assert_eq!(build_user_content(&m, false), None);
    }

    #[test]
    fn whitespace_only_returns_none() {
        let m = Message::user("   \n\t  ");
        assert_eq!(build_user_content(&m, false), None);
    }

    #[test]
    fn text_with_images_no_vision_returns_text_only() {
        let mut m = Message::user("describe this");
        m.user_image_blocks
            .push(crate::domain::message::UserImageBlock {
                mime_type: "image/png".to_string(),
                data: "base64data".to_string(),
            });
        let content = build_user_content(&m, false);
        // Vision not supported → images filtered, text remains as plain string
        assert_eq!(
            content,
            Some(serde_json::Value::String("describe this".to_string()))
        );
    }

    #[test]
    fn text_with_images_vision_returns_array() {
        let mut m = Message::user("describe this");
        m.user_image_blocks
            .push(crate::domain::message::UserImageBlock {
                mime_type: "image/png".to_string(),
                data: "base64data".to_string(),
            });
        let content = build_user_content(&m, true);
        assert!(content.is_some());
        let arr = content.unwrap();
        assert!(arr.is_array());
        let blocks = arr.as_array().unwrap();
        assert_eq!(blocks.len(), 2); // text + image
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "image");
    }

    #[test]
    fn invalid_mime_filtered() {
        let mut m = Message::user("describe");
        m.user_image_blocks
            .push(crate::domain::message::UserImageBlock {
                mime_type: "image/bmp".to_string(),
                data: "data".to_string(),
            });
        let content = build_user_content(&m, true);
        // BMP is not in ALLOWED_MIME → filtered out, only text remains
        assert_eq!(
            content,
            Some(serde_json::Value::String("describe".to_string()))
        );
    }

    #[test]
    fn images_only_no_text_vision() {
        let mut m = Message::user("");
        m.user_image_blocks
            .push(crate::domain::message::UserImageBlock {
                mime_type: "image/jpeg".to_string(),
                data: "data".to_string(),
            });
        let content = build_user_content(&m, true);
        assert!(content.is_some());
        let arr = content.unwrap();
        assert!(arr.is_array());
        assert_eq!(arr.as_array().unwrap().len(), 1); // image only
    }

    #[test]
    fn images_only_no_text_no_vision() {
        let mut m = Message::user("");
        m.user_image_blocks
            .push(crate::domain::message::UserImageBlock {
                mime_type: "image/jpeg".to_string(),
                data: "data".to_string(),
            });
        let content = build_user_content(&m, false);
        // No text, no vision → everything filtered
        assert_eq!(content, None);
    }
}
