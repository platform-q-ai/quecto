// Context pruning: the emergency ladder, the recall stubs and the spill
// manifest (#2414: the watermark pass is the only context mode; nothing
// here runs on an ordinary request).
//
// The ladder (`messages::ceiling`) stubs, then drops, the oldest messages
// only when no watermark cut can bring a request under the ceiling. All
// content spills at creation, so `recall()` retrieves stubbed or dropped
// content.
//
// Depends on: domain::message, application::sessions::use_cases (the narrow
// retention reader, D9 #1978). Never imports infrastructure or the store.

// #1046: the stub format, creation-time spill and the demotion ladder.
#[path = "context_pruning_messages.rs"]
pub mod messages;

// #2349 review M1: removal keeps call/result exchanges whole.
#[path = "context_pruning_exchanges.rs"]
mod exchanges;

use crate::application::sessions::use_cases::ListRetainedContext;
use crate::domain::message::{Message, Role};

/// Estimate token count from text content (#305, #2212).
///
/// A per-class character heuristic (`domain::token_estimate`):
///
/// - ASCII prose: ~4 chars per token (GPT cl100k_base on English);
/// - ASCII words carrying a digit, and the separator after each: ~2 chars
///   per token (numbers, hex, UUIDs, log columns: `seq` output measured
///   2.0x the prose rate in #2212);
/// - long mixed-case runs with digits (base64, JWTs, keys): ~1.4;
/// - non-ASCII: ~1 token per codepoint (CJK, emoji).
///
/// It replaced a byte-based `len/3`, which overcounted ASCII by ~33% and
/// undercounted CJK.
/// Not exact: once a provider reports its prompt size, the ceiling is
/// scaled by the observed residual (`domain::context_calibration`, 1x..4x).
pub fn estimate_tokens(text: &str) -> usize {
    crate::domain::message::Message::estimate_tokens(text)
}

pub fn estimate_total_tokens(messages: &[Message]) -> usize {
    messages.iter().map(Message::estimated_tokens).sum()
}

pub fn estimate_message_tokens(msg: &Message) -> usize {
    msg.estimated_tokens()
}

/// Truncate a string to at most `max_chars` characters, appending "..."
/// if truncated. Safe for multi-byte UTF-8 — never splits a character.
///
/// Returns `Cow::Borrowed` when the string fits (no allocation). The ellipsis
/// counts toward the budget. Bounded-scan core in [`crate::domain::text`].
pub fn truncate_utf8_safe(s: &str, max_chars: usize) -> std::borrow::Cow<'_, str> {
    crate::domain::text::truncate_chars(s, max_chars, max_chars.saturating_sub(3), "...")
}

/// Format the one-liner stub for a collapsed tool result.
pub fn collapse_stub(tool: &str, input_preview: &str, tokens: usize, spill_id: &str) -> String {
    let preview = truncate_utf8_safe(input_preview, 60);
    format!("[{tool}: {preview} ({tokens} tokens) — recall(\"{spill_id}\")]")
}

/// Replace a tool-result message's content with its compact `recall()` stub
/// (the ladder's first rung), releasing the (spilled) full content and any
/// image data. No-op if the message is not an un-collapsed tool result.
fn collapse_message(msg: &mut Message) {
    if msg.role != Role::Tool || msg.is_collapsed {
        return;
    }
    let tool_name = msg.tool_name.as_deref().unwrap_or("tool");
    let input_preview = msg.input_preview.as_deref().unwrap_or("");
    let spill_id = msg.spill_id.as_deref().unwrap_or("unknown");
    let tokens = estimate_tokens(&msg.content);
    msg.content = collapse_stub(tool_name, input_preview, tokens, spill_id);
    msg.invalidate_token_cache();
    msg.is_collapsed = true;
    // Release image data — no longer needed after collapse (spilled to disk).
    crate::domain::conversation::stored_images::release_images(msg);
}

/// Default number of most-recent turns the emergency ladder never demotes
/// (#1045).
pub const DEFAULT_PIN_RECENT_TURNS: u32 = 2;

/// Build or update the pinned, constant-size spill guidance message.
///
/// Returns `true` when durable prefix persistence must rewrite history: the
/// manifest was inserted/removed (shifting later indices), or a legacy dynamic
/// manifest was migrated in place. An already-static manifest returns `false`,
/// preserving the clean-delta fast path on ordinary tool-calling turns.
pub async fn update_spill_manifest(
    messages: &mut Vec<Message>,
    retained: &ListRetainedContext,
    session_key: &crate::domain::sessions::entities::session_identity::SessionIdentity,
) -> bool {
    let has_entries = retained.retains_entries(session_key).await.unwrap_or(false);
    if !has_entries {
        // Remove manifest if it exists and there are no entries
        let before = messages.len();
        messages.retain(|m| !m.is_manifest);
        return messages.len() != before;
    }

    let manifest = build_manifest_text();

    // Keep front-positioned guidance byte-for-byte static as the spill store
    // grows. The live index is available on demand through recall("list").
    // Find an existing manifest and migrate/update it, or insert one.
    if let Some(msg) = messages.iter_mut().find(|m| m.is_manifest) {
        if msg.content == manifest {
            false
        } else {
            msg.content = manifest;
            msg.invalidate_token_cache();
            true
        }
    } else {
        let mut msg = Message::system(manifest);
        msg.is_pinned = true;
        msg.is_manifest = true;
        // Insert after the system prompt but before conversation
        let pos = messages
            .iter()
            .position(|m| m.role != Role::System)
            .unwrap_or(messages.len());
        messages.insert(pos, msg);
        true
    }
}

/// Build static front-positioned session-memory guidance.
///
/// No entry-derived bytes may appear here because provider prompt caches use
/// exact prefix matching. The complete dynamic index is `recall("list")`.
pub fn build_manifest_text() -> String {
    "[Session memory is available via recall()]\n\
     Use recall(\"list\") for the full session-memory index, then recall(\"<id>\") to retrieve content."
        .to_string()
}

#[cfg(test)]
#[path = "context_pruning_tests.rs"]
mod tests;
// #951: spilling ceiling + tail-pinning tests live in a separate file to
// respect the 750-line source cap.
#[cfg(test)]
#[path = "context_pruning_spill_tests.rs"]
mod spill_tests;

// #1046: message-collapse + ladder + creation-spill tests (same cap rule).
#[cfg(test)]
#[path = "context_pruning_message_tests.rs"]
mod message_tests;

// PR #1048: unspilled-content (spill_id == None) safety tests (same cap rule).
#[cfg(test)]
#[path = "context_pruning_unspilled_tests.rs"]
mod unspilled_tests;
