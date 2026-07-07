// Context pruning: tool-call collapse + sliding-window enforcement with spill-to-disk.
//
// Once the number of tool-result messages in the session exceeds
// `context_collapse_after_tool_calls` (default 50), the oldest tool results are
// collapsed to compact `recall(spill_id)` stubs. Independently, messages age out
// and are dropped by `enforce_context_ceiling_spilling()` when the conversation
// exceeds the token budget. Tool outputs spill to disk at creation time; dropped
// conversation messages spill at drop time (#951), so `recall()` can retrieve
// collapsed or dropped content.
//
// Depends on: domain::message, domain::session (ContextSpillStore).
// Never imports infrastructure.

use std::fmt::Write;

use crate::domain::message::{Message, Role};
use crate::domain::session::{ContextSpillStore, SpillEntry, SpillIndex};

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
    let live_tool_calls = messages
        .iter()
        .filter(|m| m.role == Role::Tool && !m.is_collapsed)
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
        if msg.role != Role::Tool || msg.is_collapsed {
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

/// Default number of most-recent turns the spilling ceiling never drops.
pub const DEFAULT_PIN_RECENT_TURNS: u32 = 2;

/// Enforce the context ceiling with tail-pinning, returning the dropped
/// messages so the caller can spill their content before it is lost (#951).
///
/// The token budget covers both text content and base64 image blocks,
/// so image-heavy sessions are correctly bounded. Note that this is an
/// *application-level* budget — it does not know the actual model context
/// window. Users on smaller-context models (e.g. GPT-4 128k) should
/// override `max_context_tokens` in their config.
///
/// Contract:
/// - Never drops pinned messages (system prompt, manifest).
/// - Never drops the in-flight user prompt — the last *turn-less* user
///   message (production never turn-stamps user prompts; a mid-prompt
///   malformed-feedback user message IS stamped, see
///   `append_malformed_feedback`, so it can never masquerade as the prompt).
/// - Within the current prompt (everything from the in-flight prompt
///   onward), never drops turn-less messages (not yet stamped) nor messages
///   in the `pin_recent_turns` most recent distinct turns. Turn numbering
///   restarts on each prompt, so earlier prompts' turns stay droppable.
/// - Everything before the in-flight prompt — including fully turn-less
///   histories saved by older builds — is droppable unless pinned, so the
///   ceiling is still enforced on resumed pre-turn-stamping sessions.
/// - Returns every dropped message (oldest first) so the caller can
///   write non-tool content to the spill store with id `turn{N}:msg:{role}`.
pub fn enforce_context_ceiling_spilling(
    messages: &mut Vec<Message>,
    max_tokens: usize,
    pin_recent_turns: u32,
) -> Vec<Message> {
    if estimate_total_tokens(messages) <= max_tokens {
        return Vec::new();
    }
    // The in-flight user prompt: the last turn-less user message. Everything
    // from it onward belongs to the prompt currently being processed.
    let region_start = messages
        .iter()
        .rposition(|m| m.role == Role::User && m.turn.is_none())
        .unwrap_or(0);
    let mut recent_turns: Vec<u32> = messages[region_start..]
        .iter()
        .filter_map(|m| m.turn)
        .collect();
    recent_turns.sort_unstable();
    recent_turns.dedup();
    let keep_from = recent_turns.len().saturating_sub(pin_recent_turns as usize);
    let pinned_turns = &recent_turns[keep_from..];

    let droppable: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|&(i, m)| {
            !m.is_pinned
                && if i < region_start {
                    true // earlier prompts: droppable (turn-stamped or not)
                } else {
                    // Current prompt: only stamped turns outside the pinned
                    // tail are droppable. The prompt itself and any not-yet-
                    // stamped in-flight message are turn-less → kept.
                    m.turn.is_some_and(|t| !pinned_turns.contains(&t))
                }
        })
        .map(|(i, _)| i)
        .collect();
    drop_until_under_budget(messages, max_tokens, &droppable)
}

/// Enforce the spilling context ceiling and write every dropped conversation
/// message (assistant/user) to the spill store as `turn{N}:msg:{role}` so it
/// stays recallable (#951). Tool results are skipped — they were already
/// spilled at creation time under `turn{N}:{tool}:{idx}`.
///
/// Turn numbering restarts on every prompt while the spill file persists for
/// the whole session, so the base id alone would collide across prompts (and
/// turn-less user prompts would all collide on `turn0:msg:user`). The spill
/// store is append-only and `recall` returns the first match, so a collision
/// would make later content unreachable. Ids are therefore de-duplicated
/// against the store's index with a `:{n}` suffix (`turn1:msg:assistant`,
/// `turn1:msg:assistant:2`, ...).
///
/// Returns `(dropped_count, message_spilled)`; `message_spilled` tells the
/// caller the manifest needs a refresh even on a turn with no tool calls.
pub async fn enforce_context_ceiling_spilling_to_store(
    messages: &mut Vec<Message>,
    max_tokens: usize,
    pin_recent_turns: u32,
    spill_store: &dyn ContextSpillStore,
    session_key: &str,
) -> (usize, bool) {
    let dropped = enforce_context_ceiling_spilling(messages, max_tokens, pin_recent_turns);
    let dropped_count = dropped.len();
    let mut spilled = false;
    let mut existing_ids: Option<std::collections::HashSet<String>> = None;
    for msg in dropped {
        if msg.role == Role::Tool || msg.content.is_empty() {
            continue;
        }
        // Lazily fetch the store index once, only when something will spill.
        let ids = match existing_ids {
            Some(ref mut ids) => ids,
            None => {
                let seeded = spill_store
                    .list_entries(session_key)
                    .await
                    .map(|entries| entries.iter().map(|e| e.id.clone()).collect())
                    .unwrap_or_default();
                existing_ids.insert(seeded)
            }
        };
        let role = msg.role.as_str();
        let base = format!("turn{}:msg:{role}", msg.turn.unwrap_or(0));
        let mut id = base.clone();
        let mut n = 1usize;
        while ids.contains(&id) {
            n += 1;
            id = format!("{base}:{n}");
        }
        ids.insert(id.clone());
        let entry = SpillEntry {
            id,
            tool: role.to_string(),
            input_preview: truncate_utf8_safe(&msg.content, 100).into_owned(),
            tokens: estimate_tokens(&msg.content),
            content: msg.content,
        };
        match spill_store.append(session_key, &entry).await {
            Ok(()) => spilled = true,
            Err(e) => {
                tracing::warn!(
                    target: "context_prune",
                    error = %e,
                    "failed to spill dropped message"
                );
            }
        }
    }
    (dropped_count, spilled)
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
    fn test_enforce_context_ceiling_under_budget() {
        let mut messages = vec![Message::user("short")];
        let dropped = enforce_context_ceiling_spilling(&mut messages, 1000, 2).len();
        assert_eq!(dropped, 0);
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_enforce_context_ceiling_drops_oldest() {
        // Create messages that exceed budget
        let big_content = "x".repeat(600); // ~150 tokens
        let mut messages = vec![
            Message::user(&big_content),
            Message::user(&big_content),
            Message::user(&big_content),
        ];
        // Budget of 250 tokens, total ~450 tokens. The trailing user message
        // is the in-flight prompt (kept); the two older ones must go.
        let dropped = enforce_context_ceiling_spilling(&mut messages, 250, 2).len();
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
        let dropped = enforce_context_ceiling_spilling(&mut messages, 250, 2).len();
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
        let dropped = enforce_context_ceiling_spilling(&mut messages, 150, 2).len();
        assert_eq!(dropped, 1);
        assert_eq!(messages.len(), 1);
        assert!(estimate_total_tokens(&messages) <= 150);
    }

    #[test]
    fn test_collapse_disabled_constant() {
        assert_eq!(COLLAPSE_DISABLED, u32::MAX);
    }

    // --- #1017: collapse triggers on number of tool calls, default 50 ---

    fn tool_call_msg(i: u32) -> Message {
        let mut m = Message::tool(format!("call_{i}"), format!("output {i}"));
        m.turn = Some(i);
        m.tool_name = Some("bash".to_string());
        m.spill_id = Some(format!("turn{i}:bash:0"));
        m
    }

    #[test]
    fn collapse_over_limit_keeps_most_recent_n_tool_calls() {
        // 60 tool calls, keep the most recent 50 → collapse the oldest 10.
        let mut messages: Vec<Message> = (1..=60).map(tool_call_msg).collect();
        let collapsed = collapse_tool_results_over_limit(&mut messages, 50);
        assert_eq!(collapsed, 10);
        for msg in &messages[..10] {
            assert!(msg.is_collapsed, "oldest 10 tool results must be collapsed");
        }
        for msg in &messages[10..] {
            assert!(
                !msg.is_collapsed,
                "the 50 most recent tool results must stay full"
            );
        }
    }

    #[test]
    fn collapse_over_limit_triggers_at_one_past_the_threshold() {
        // Exactly threshold+1 tool calls → the single oldest is collapsed.
        let mut messages: Vec<Message> = (1..=51).map(tool_call_msg).collect();
        let collapsed = collapse_tool_results_over_limit(&mut messages, 50);
        assert_eq!(collapsed, 1);
        assert!(messages[0].is_collapsed);
    }

    #[test]
    fn collapse_over_limit_no_collapse_at_or_under_threshold() {
        // Exactly the threshold count → nothing collapses.
        let mut messages: Vec<Message> = (1..=50).map(tool_call_msg).collect();
        let collapsed = collapse_tool_results_over_limit(&mut messages, 50);
        assert_eq!(collapsed, 0);
        assert!(messages.iter().all(|m| !m.is_collapsed));

        // Positive control: the SAME message set with one more tool call must
        // collapse exactly one, so this test can distinguish the real trigger
        // from a no-op implementation.
        messages.push(tool_call_msg(51));
        let collapsed = collapse_tool_results_over_limit(&mut messages, 50);
        assert_eq!(collapsed, 1);
        assert!(messages[0].is_collapsed);
    }

    #[test]
    fn collapse_over_limit_honors_non_default_threshold() {
        // The threshold is configurable, not hard-coded to 50: at limit 10,
        // 10 tool calls collapse nothing but 11 collapse exactly one.
        let mut at_limit: Vec<Message> = (1..=10).map(tool_call_msg).collect();
        assert_eq!(collapse_tool_results_over_limit(&mut at_limit, 10), 0);
        assert!(at_limit.iter().all(|m| !m.is_collapsed));

        let mut over_limit: Vec<Message> = (1..=11).map(tool_call_msg).collect();
        assert_eq!(collapse_tool_results_over_limit(&mut over_limit, 10), 1);
        assert!(over_limit[0].is_collapsed);
        assert!(over_limit[1..].iter().all(|m| !m.is_collapsed));
    }

    #[test]
    fn collapse_over_limit_is_cumulative_across_prompts() {
        // Two separate prompts (turn numbers reset to a small range each time,
        // mirroring a fresh run_loop). The trigger must count tool-result
        // messages across the whole session, not per-prompt turns.
        let mut messages: Vec<Message> = Vec::new();
        for i in 1..=30u32 {
            messages.push(Message::user("prompt"));
            messages.push(tool_call_msg((i % 3) + 1)); // small, repeating turns
        }
        for i in 1..=25u32 {
            messages.push(Message::user("prompt"));
            messages.push(tool_call_msg((i % 3) + 1)); // turns reset again
        }
        // 55 tool calls total, threshold 50 → oldest 5 collapse.
        let collapsed = collapse_tool_results_over_limit(&mut messages, 50);
        assert_eq!(collapsed, 5);
    }

    #[test]
    fn collapse_over_limit_disabled_by_sentinel() {
        // The sentinel disables collapse even when the live tool-call count
        // (100) would collapse under any finite threshold.
        let mut messages: Vec<Message> = (1..=100).map(tool_call_msg).collect();
        let collapsed = collapse_tool_results_over_limit(&mut messages, COLLAPSE_DISABLED);
        assert_eq!(collapsed, 0);
        assert!(messages.iter().all(|m| !m.is_collapsed));

        // Control: the identical message set DOES collapse at a finite limit,
        // so only the sentinel short-circuit can explain the 0 above (a pure
        // count check would have collapsed 50 here too).
        let collapsed = collapse_tool_results_over_limit(&mut messages, 50);
        assert_eq!(collapsed, 50);
    }

    #[test]
    fn collapse_over_limit_uses_recall_stub_and_clears_images() {
        use crate::domain::tool::ImageBlock;
        let mut messages: Vec<Message> = (1..=51).map(tool_call_msg).collect();
        messages[0].image_blocks = vec![ImageBlock {
            mime_type: "image/png",
            data: "x".repeat(40),
        }];
        collapse_tool_results_over_limit(&mut messages, 50);
        assert!(messages[0].is_collapsed);
        assert!(
            messages[0].content.contains("recall(\"turn1:bash:0\")"),
            "collapsed output must be replaced by the recall() stub"
        );
        assert!(
            messages[0].image_blocks.is_empty(),
            "collapsed tool result must release image data"
        );
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

// #951: spilling ceiling + tail-pinning tests live in a separate file to
// respect the 750-line source cap.
#[cfg(test)]
#[path = "context_pruning_spill_tests.rs"]
mod spill_tests;
