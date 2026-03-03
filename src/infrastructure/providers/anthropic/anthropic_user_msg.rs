use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

/// Returns `true` if the cancel flag is set (checked with Relaxed ordering).
pub(super) fn is_cancelled(flag: &Option<Arc<AtomicBool>>) -> bool {
    flag.as_ref()
        .map(|f| f.load(Ordering::Relaxed))
        .unwrap_or(false)
}

/// User message content block builder for the Anthropic API (#188).
///
/// Handles structured content block arrays for user messages containing inline
/// images, model capability filtering, and empty-content skipping.
use crate::domain::message::Message;

/// Returns `true` for models known to support image inputs.
///
/// Defaults to `true` for unknown models (fail-open). Legacy models that
/// predate vision support are explicitly denied. Uses exact match to avoid
/// false positives from future models with similar prefixes (e.g. "claude-2025-x").
pub(super) fn model_supports_vision(model: &str) -> bool {
    // Exact model identifiers known NOT to support vision.
    const NON_VISION: &[&str] = &[
        "claude-instant-1",
        "claude-instant-1.2",
        "claude-2",
        "claude-2.0",
        "claude-2.1",
    ];
    !NON_VISION.contains(&model)
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
