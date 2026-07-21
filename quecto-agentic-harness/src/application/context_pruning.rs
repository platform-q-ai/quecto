// Context pruning: tool-call collapse + sliding-window enforcement with spill-to-disk.
//
// Once the number of tool-result messages in the session exceeds
// `context_collapse_after_tool_calls` (default 50), the oldest tool results are
// collapsed to compact `recall(spill_id)` stubs. Conversation messages get the
// symmetric lifecycle in the `messages` submodule (#1046): spilled at creation,
// count-collapsed via `context_collapse_after_messages`, and demoted down the
// ladder (stub → drop) when the conversation exceeds the token budget. All
// content spills to disk at creation time, so `recall()` can retrieve
// collapsed or dropped content.
//
// Depends on: domain::message, domain::session (ContextSpillStore).
// Never imports infrastructure.

// #1046: conversation-message collapse, demotion ladder, creation-time spill.
#[path = "context_pruning_messages.rs"]
pub mod messages;

use crate::domain::message::{Message, Role};
use crate::domain::session::ContextSpillStore;

/// Sentinel value indicating that tool-result collapse is disabled.
/// When `context_collapse_after_tool_calls` is set to this value, collapse is
/// short-circuited: a session would need `u32::MAX` tool-result messages before
/// the count-based trigger could fire, which is unreachable in practice.
pub const COLLAPSE_DISABLED: u32 = u32::MAX;

/// Estimate token count from text content (#305).
///
/// Uses a two-class character heuristic that is accurate for both ASCII prose
/// and non-ASCII (CJK, emoji, etc.):
///
/// - ASCII codepoints: ~4 chars per token (matches GPT cl100k_base for English)
/// - Non-ASCII codepoints: ~1 char per token (CJK, emoji, etc. are typically
///   1 token per codepoint in current tokenisers)
///
/// The old byte-based estimate (`len/3`) overcounted ASCII by ~33% and gave
/// the same token count for 100 CJK chars as for 300 ASCII chars, which is
/// inaccurate in opposite directions. This heuristic is more balanced: it
/// reduces pruning pressure on ASCII-heavy sessions without undercounting CJK
/// (which would weaken pruning as a prompt-injection defence).
///
/// The estimate is intentionally slightly conservative — it is better to
/// prune a turn early than to exceed the provider's context limit.
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

/// Replace a tool-result message's content with its compact `recall()` stub,
/// releasing the (spilled) full content and any image data. No-op if the
/// message is not an un-collapsed tool result.
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
    msg.image_blocks.clear();
}

/// Collapse the oldest tool results once the number of tool calls in the
/// session exceeds `max_tool_calls`, keeping only the most recent
/// `max_tool_calls` tool results in full context (#1017).
///
/// The trigger is the cumulative **number of un-collapsed tool-result messages**
/// in the conversation, so it accumulates across prompts within a session
/// (message history persists) rather than resetting each `run_loop` invocation.
/// Already-collapsed results are not counted (they no longer weigh on context),
/// so the collapse front advances monotonically as new tool calls arrive.
///
/// `max_tool_calls == COLLAPSE_DISABLED` (`u32::MAX`) disables collapse.
/// Returns the number of tool results collapsed.
pub fn collapse_tool_results_over_limit(messages: &mut [Message], max_tool_calls: u32) -> usize {
    if max_tool_calls == COLLAPSE_DISABLED {
        return 0;
    }
    // spill_id == None means the output never reached the spill store (append
    // failure / missing store): collapsing it would mint an unresolvable
    // recall() stub, so such results are excluded from both the count and the
    // collapse front (same rule as the conversation-message trigger).
    let live_tool_calls = messages
        .iter()
        .filter(|m| m.role == Role::Tool && !m.is_collapsed && m.spill_id.is_some())
        .count();
    let mut to_collapse = live_tool_calls.saturating_sub(max_tool_calls as usize);
    if to_collapse == 0 {
        return 0;
    }
    let mut collapsed = 0;
    for msg in messages.iter_mut() {
        if to_collapse == 0 {
            break;
        }
        if msg.role != Role::Tool || msg.is_collapsed || msg.spill_id.is_none() {
            continue;
        }
        collapse_message(msg);
        collapsed += 1;
        to_collapse -= 1;
    }
    collapsed
}

/// Walk `droppable` (oldest first) marking messages for removal until the
/// running total fits `max_tokens`, then remove them in a single pass.
/// Returns the removed messages, oldest first. `droppable` must be sorted
/// ascending (it is built by an in-order scan).
fn drop_until_under_budget(
    messages: &mut Vec<Message>,
    max_tokens: usize,
    droppable: &[usize],
) -> Vec<Message> {
    let mut total = estimate_total_tokens(messages);
    let mut drop_count = 0;
    for &idx in droppable {
        if total <= max_tokens {
            break;
        }
        total = total.saturating_sub(estimate_message_tokens(&messages[idx]));
        drop_count += 1;
    }
    if drop_count == 0 {
        return Vec::new();
    }
    // Only the first `drop_count` droppable entries go. Sorted slice +
    // binary_search gives O(log n) lookup without a HashSet.
    let drop_indices = &droppable[..drop_count];
    let mut dropped = Vec::with_capacity(drop_count);
    let mut kept = Vec::with_capacity(messages.len() - drop_count);
    for (idx, msg) in std::mem::take(messages).into_iter().enumerate() {
        if drop_indices.binary_search(&idx).is_ok() {
            dropped.push(msg);
        } else {
            kept.push(msg);
        }
    }
    *messages = kept;
    dropped
}

/// Default number of most-recent turns the demotion-ladder ceiling never
/// demotes (#1045).
pub const DEFAULT_PIN_RECENT_TURNS: u32 = 2;

/// Build or update the pinned, constant-size spill guidance message.
///
/// Returns `true` when durable prefix persistence must rewrite history: the
/// manifest was inserted/removed (shifting later indices), or a legacy dynamic
/// manifest was migrated in place. An already-static manifest returns `false`,
/// preserving the clean-delta fast path on ordinary tool-calling turns.
pub async fn update_spill_manifest(
    messages: &mut Vec<Message>,
    spill_store: &dyn ContextSpillStore,
    session_key: &str,
) -> bool {
    let has_entries = spill_store.has_entries(session_key).await.unwrap_or(false);
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
