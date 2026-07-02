// Context pruning: sliding-window enforcement with spill-to-disk.
//
// Tool results are no longer collapsed after N turns. Instead, they age
// naturally and are dropped by `enforce_context_ceiling()` when the
// conversation exceeds the token budget. Spill-to-disk still happens at
// creation time so `recall()` can retrieve dropped outputs.
//
// Depends on: domain::message, domain::session (ContextSpillStore).
// Never imports infrastructure.

use std::fmt::Write;

use crate::domain::message::{Message, Role};
use crate::domain::session::{ContextSpillStore, SpillIndex};

/// Sentinel value indicating that tool-result collapse is disabled.
/// Used as the default for `context_collapse_after_turns`. Safe because
/// `max_tool_iterations` (999_999) is far below `u32::MAX`, so
/// `current_turn.saturating_sub(turn)` can never reach this threshold.
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

/// Collapse tool results older than `collapse_after` turns.
/// Returns the number of tool results collapsed.
///
/// With the default `collapse_after = COLLAPSE_DISABLED` (u32::MAX), this
/// function is never called — the agent loop short-circuits it. It remains
/// available for users who explicitly set a lower `context_collapse_after_turns`
/// value in their config.
pub fn collapse_old_tool_results(
    messages: &mut [Message],
    current_turn: u32,
    collapse_after: u32,
) -> usize {
    let mut collapsed = 0;
    for msg in messages.iter_mut() {
        if msg.role != Role::Tool || msg.is_collapsed {
            continue;
        }
        if let Some(turn) = msg.turn {
            if current_turn.saturating_sub(turn) >= collapse_after {
                let tool_name = msg.tool_name.as_deref().unwrap_or("tool");
                let input_preview = msg.input_preview.as_deref().unwrap_or("");
                let spill_id = msg.spill_id.as_deref().unwrap_or("unknown");
                let tokens = estimate_tokens(&msg.content);
                msg.content = collapse_stub(tool_name, input_preview, tokens, spill_id);
                msg.invalidate_token_cache();
                msg.is_collapsed = true;
                // Release image data — no longer needed after collapse (spilled to disk).
                msg.image_blocks.clear();
                collapsed += 1;
            }
        }
    }
    collapsed
}

/// Enforce a hard ceiling on total context tokens.
/// Drops oldest non-pinned messages until under budget.
/// Returns the number of messages dropped.
///
/// The token budget covers both text content and base64 image blocks,
/// so image-heavy sessions are correctly bounded. Note that this is an
/// *application-level* budget — it does not know the actual model context
/// window. Users on smaller-context models (e.g. GPT-4 128k) should
/// override `max_context_tokens` in their config.
///
/// Uses a two-pass approach to avoid O(n^2) repeated scanning:
/// 1. Calculate total tokens and identify droppable message indices.
/// 2. Walk droppable indices from oldest, marking for removal until under budget.
/// 3. Single retain() pass to remove all marked messages.
pub fn enforce_context_ceiling(messages: &mut Vec<Message>, max_tokens: usize) -> usize {
    let mut total = estimate_total_tokens(messages);
    if total <= max_tokens {
        return 0;
    }

    // Collect indices of droppable messages (oldest first, already in order).
    // Then count how many to drop from the front until under budget.
    let droppable: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.is_pinned)
        .map(|(i, _)| i)
        .collect();

    let mut drop_count = 0;
    for &idx in &droppable {
        if total <= max_tokens {
            break;
        }
        total = total.saturating_sub(estimate_message_tokens(&messages[idx]));
        drop_count += 1;
    }

    if drop_count == 0 {
        return 0;
    }

    // Build a set of indices to drop (only the first `drop_count` droppable entries).
    // Use a sorted slice + binary_search for O(log n) lookup instead of HashSet.
    let drop_indices = &droppable[..drop_count];
    let mut idx = 0;
    messages.retain(|_| {
        let keep = drop_indices.binary_search(&idx).is_err();
        idx += 1;
        keep
    });
    drop_count
}

/// Build or update the pinned spill manifest message.
/// Shows the last 10 spill entries plus summary metadata.
/// Fixed token budget (~500 tokens) regardless of session length.
pub async fn update_spill_manifest(
    messages: &mut Vec<Message>,
    spill_store: &dyn ContextSpillStore,
    session_key: &str,
) {
    let entries = spill_store
        .list_entries(session_key)
        .await
        .unwrap_or_default();
    if entries.is_empty() {
        // Remove manifest if it exists and there are no entries
        messages.retain(|m| !m.is_manifest);
        return;
    }

    let manifest = build_manifest_text(&entries);

    // Find existing manifest message and update, or insert one
    if let Some(msg) = messages.iter_mut().find(|m| m.is_manifest) {
        msg.content = manifest;
        msg.invalidate_token_cache();
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
    }
}

/// Build the manifest text from spill index entries.
pub fn build_manifest_text(entries: &[SpillIndex]) -> String {
    let total = entries.len();
    let oldest = &entries[0];
    let latest = &entries[total - 1];
    let recent: Vec<_> = entries.iter().rev().take(10).collect();

    let mut manifest = format!(
        "[Session memory: {} spilled entries via recall()]\n\
         Oldest: {} — {} ({} tokens)\n\
         Latest: {} — {} ({} tokens)\n\
         Recent:\n",
        total,
        oldest.id,
        oldest.input_preview,
        oldest.tokens,
        latest.id,
        latest.input_preview,
        latest.tokens,
    );
    for entry in recent.iter().rev() {
        let _ = writeln!(
            manifest,
            "  {} — {} ({} tokens)",
            entry.id, entry.input_preview, entry.tokens
        );
    }
    manifest.push_str("Use recall(\"<id>\") to retrieve. Use recall(\"list\") for full index.");
    manifest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::message::Message;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1); // 4 chars / 4 = 1
        assert_eq!(estimate_tokens("abcdefgh"), 2); // 8 / 4 = 2
        assert_eq!(estimate_tokens("ab"), 1); // ceiling: div_ceil(2, 4) = 1
        // 400 ASCII chars → 100 tokens at 4 chars/token
        let large = "x".repeat(400);
        assert_eq!(estimate_tokens(&large), 100);
    }

    #[test]
    fn test_truncate_utf8_safe() {
        assert_eq!(truncate_utf8_safe("hello", 10), "hello");
        assert_eq!(truncate_utf8_safe("hello world", 8), "hello...");
        // Multi-byte UTF-8
        let emoji = "🎉🎊🎈🎁🎂";
        let result = truncate_utf8_safe(emoji, 4);
        assert!(result.ends_with("..."));
        assert!(result.chars().count() <= 4);
    }

    #[test]
    fn test_collapse_stub_format() {
        let stub = collapse_stub(
            "bash",
            "find ~/.local/share -type d",
            19156,
            "turn20:bash:0",
        );
        assert!(stub.contains("[bash:"));
        assert!(stub.contains("19156 tokens"));
        assert!(stub.contains("recall(\"turn20:bash:0\")"));
    }

    #[test]
    fn test_tool_results_never_collapsed_regardless_of_age() {
        let mut messages = vec![Message::user("test"), {
            let mut m = Message::tool("call_1", "big output here");
            m.turn = Some(1);
            m.tool_name = Some("bash".to_string());
            m.spill_id = Some("turn1:bash:0".to_string());
            m
        }];
        // Even at turn 100, tool results should NOT be collapsed
        // collapse_after = u32::MAX effectively disables collapse
        let collapsed = collapse_old_tool_results(&mut messages, 100, u32::MAX);
        assert_eq!(collapsed, 0);
        assert!(!messages[1].is_collapsed);
        assert_eq!(messages[1].content, "big output here");
    }

    #[test]
    fn test_tool_results_stay_full_across_many_turns() {
        let mut messages = vec![];
        // Create tool results on turns 1-10
        for turn in 1..=10u32 {
            let mut m = Message::tool(format!("call_{turn}"), format!("output for turn {turn}"));
            m.turn = Some(turn);
            m.tool_name = Some("bash".to_string());
            m.spill_id = Some(format!("turn{turn}:bash:0"));
            messages.push(m);
        }
        // Run collapse with u32::MAX (disabled) at turn 20
        let collapsed = collapse_old_tool_results(&mut messages, 20, u32::MAX);
        assert_eq!(collapsed, 0);
        // All messages should still have their original content
        for (i, msg) in messages.iter().enumerate() {
            let turn = i as u32 + 1;
            assert!(!msg.is_collapsed);
            assert_eq!(msg.content, format!("output for turn {turn}"));
        }
    }

    #[test]
    fn test_user_assistant_system_never_collapsed() {
        let mut messages = vec![
            {
                let mut m = Message::system("system prompt");
                m.turn = Some(0);
                m
            },
            {
                let mut m = Message::user("user input");
                m.turn = Some(0);
                m
            },
            {
                let mut m = Message::assistant("assistant response", vec![]);
                m.turn = Some(0);
                m
            },
        ];
        let collapsed = collapse_old_tool_results(&mut messages, 100, u32::MAX);
        assert_eq!(collapsed, 0);
    }

    #[test]
    fn test_enforce_context_ceiling_under_budget() {
        let mut messages = vec![Message::user("short")];
        let dropped = enforce_context_ceiling(&mut messages, 1000);
        assert_eq!(dropped, 0);
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_enforce_context_ceiling_drops_oldest() {
        // Create messages that exceed budget
        let big_content = "x".repeat(600); // ~200 tokens
        let mut messages = vec![
            Message::user(&big_content),
            Message::user(&big_content),
            Message::user(&big_content),
        ];
        // Budget of 250 tokens, total ~600 tokens. Need to drop 2.
        let dropped = enforce_context_ceiling(&mut messages, 250);
        assert!(dropped >= 1);
        assert!(estimate_total_tokens(&messages) <= 250);
    }

    #[test]
    fn test_enforce_context_ceiling_preserves_pinned() {
        let big = "x".repeat(600);
        let mut messages = vec![
            Message::system("system prompt"), // pinned by default
            Message::user(&big),
            Message::user(&big),
        ];
        let dropped = enforce_context_ceiling(&mut messages, 250);
        assert!(dropped > 0);
        // System message (pinned) should still be there
        assert!(messages.iter().any(|m| m.role == Role::System));
    }

    #[test]
    fn test_build_manifest_text() {
        let entries = vec![
            SpillIndex {
                id: "turn1:bash:0".to_string(),
                tool: "bash".to_string(),
                input_preview: "echo hello".to_string(),
                tokens: 100,
            },
            SpillIndex {
                id: "turn2:bash:0".to_string(),
                tool: "bash".to_string(),
                input_preview: "ls -la".to_string(),
                tokens: 200,
            },
        ];
        let text = build_manifest_text(&entries);
        assert!(text.contains("2 spilled entries"));
        assert!(text.contains("turn1:bash:0"));
        assert!(text.contains("turn2:bash:0"));
        assert!(text.contains("recall(\"<id>\")"));
        assert!(text.contains("recall(\"list\")"));
    }

    #[test]
    fn test_estimate_total_tokens() {
        let messages = vec![
            Message::user("abc"),    // 1 token
            Message::user("abcdef"), // 2 tokens
        ];
        assert_eq!(estimate_total_tokens(&messages), 3);
    }

    #[test]
    fn test_estimate_message_tokens_includes_image_blocks() {
        use crate::domain::tool::ImageBlock;
        let mut msg = Message::tool("call_1", "abc"); // div_ceil(3,4)=1 token text
        msg.image_blocks = vec![ImageBlock {
            mime_type: "image/png",
            data: "x".repeat(300), // div_ceil(300,4)=75 tokens image
        }];
        // 1 text + 75 image + 2 for tool_call_id "call_1" (div_ceil(6,4)=2)
        assert_eq!(estimate_message_tokens(&msg), 78);
    }

    #[test]
    fn test_enforce_context_ceiling_accounts_for_image_blocks() {
        use crate::domain::tool::ImageBlock;
        let mut msg1 = Message::tool("call_1", "abc");
        msg1.image_blocks = vec![ImageBlock {
            mime_type: "image/png",
            data: "x".repeat(600), // 200 tokens
        }];
        let msg2 = Message::user("y".repeat(300)); // 100 tokens
        let mut messages = vec![msg1, msg2];
        // Budget of 150: total is ~301 tokens, should drop oldest (the 200-token image msg)
        let dropped = enforce_context_ceiling(&mut messages, 150);
        assert_eq!(dropped, 1);
        assert_eq!(messages.len(), 1);
        assert!(estimate_total_tokens(&messages) <= 150);
    }

    #[test]
    fn test_collapse_disabled_constant() {
        assert_eq!(COLLAPSE_DISABLED, u32::MAX);
    }

    // --- #305: Improved token estimation heuristic ---

    #[test]
    fn estimate_tokens_ascii_prose_uses_four_chars_per_token() {
        // 400 ASCII chars → 100 tokens at 4 chars/token
        let prose = "a".repeat(400);
        assert_eq!(estimate_tokens(&prose), 100);
    }

    #[test]
    fn estimate_tokens_ascii_ceiling_division() {
        // div_ceil(300, 4) = 75 tokens for 300 ASCII chars
        let text = "x_".repeat(150); // 300 ASCII chars
        assert_eq!(estimate_tokens(&text), 75);
    }

    #[test]
    fn estimate_tokens_cjk_one_token_per_char() {
        // CJK chars use the non-ASCII branch: 1 token per codepoint.
        // 100 CJK chars → 100 tokens (accurate: GPT tokeniser gives ~1 token/CJK char).
        // This is better than the old byte heuristic: 300 bytes/3 = 100 (same answer,
        // but correct reasoning). For pure ASCII, bytes/3 overcounted; chars/4 is accurate.
        let cjk = "中".repeat(100); // 100 non-ASCII codepoints
        assert_eq!(estimate_tokens(&cjk), 100); // 100 * 1 = 100
    }

    #[test]
    fn estimate_tokens_mixed_ascii_and_cjk() {
        // 8 ASCII chars → div_ceil(8,4)=2 tokens; 3 CJK chars → 3 tokens = 5 total
        let mixed = "hello!! 中文日";
        assert_eq!(
            estimate_tokens(mixed),
            estimate_tokens("hello!! ") + estimate_tokens("中文日")
        );
    }

    #[test]
    fn estimate_tokens_empty_string_is_zero() {
        assert_eq!(estimate_tokens(""), 0);
    }
}
