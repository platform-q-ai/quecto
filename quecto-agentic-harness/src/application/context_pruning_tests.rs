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
fn test_ceiling_ladder_under_budget_is_a_no_op() {
    let mut messages = vec![Message::user("short")];
    let outcome = messages::enforce_context_ceiling_ladder(&mut messages, 1000, 2);
    assert_eq!(outcome.collapsed_to_stubs, 0);
    assert_eq!(outcome.dropped, 0);
    assert_eq!(messages.len(), 1);
}

#[test]
fn test_ceiling_ladder_demotes_oldest_to_meet_budget() {
    // Create messages that exceed budget, spilled at creation as
    // production guarantees (#1046 AC1) — unspilled content is never
    // stubbed (PR #1048).
    let big_content = "x".repeat(600); // ~150 tokens
    let mut messages: Vec<Message> = (0..3)
        .map(|i| {
            let mut m = Message::user(&big_content);
            m.spill_id = Some(format!("turn{}:msg:user", i + 1));
            m
        })
        .collect();
    // Budget of 250 tokens, total ~450 tokens. The trailing user message
    // is the in-flight prompt (kept); older ones demote until it fits.
    let outcome = messages::enforce_context_ceiling_ladder(&mut messages, 250, 2);
    assert!(outcome.collapsed_to_stubs >= 1);
    assert!(estimate_total_tokens(&messages) <= 250);
}

#[test]
fn test_ceiling_ladder_preserves_pinned() {
    let big = "x".repeat(600);
    let mut old_user = Message::user(&big);
    old_user.spill_id = Some("turn1:msg:user".into());
    let mut messages = vec![
        Message::system("system prompt"), // pinned by default
        old_user,
        Message::user(&big),
    ];
    let outcome = messages::enforce_context_ceiling_ladder(&mut messages, 250, 2);
    assert!(outcome.collapsed_to_stubs > 0);
    // System message (pinned) should still be there, untouched.
    let system = messages.iter().find(|m| m.role == Role::System).unwrap();
    assert!(!system.is_collapsed);
}

#[test]
fn test_build_manifest_text() {
    assert_eq!(
        build_manifest_text(),
        "[Session memory is available via recall()]\n\
             Use recall(\"list\") for the full session-memory index, then recall(\"<id>\") to retrieve content."
    );
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
fn test_ceiling_ladder_accounts_for_image_blocks() {
    use crate::domain::tool::ImageBlock;
    let mut msg1 = Message::tool("call_1", "abc");
    // Spilled at creation, like every production tool result — the ladder
    // only stubs spill-backed content (unspilled => recall() would dangle).
    msg1.spill_id = Some("turn1:tool:0".into());
    msg1.image_blocks = vec![ImageBlock {
        mime_type: "image/png",
        data: "x".repeat(600), // 200 tokens
    }];
    let msg2 = Message::user("y".repeat(300)); // 100 tokens
    let mut messages = vec![msg1, msg2];
    // Budget of 150: total is ~301 tokens; the image-heavy tool result
    // must be demoted (its stub releases the image data) to fit.
    let outcome = messages::enforce_context_ceiling_ladder(&mut messages, 150, 2);
    assert_eq!(outcome.collapsed_to_stubs, 1);
    assert!(estimate_total_tokens(&messages) <= 150);
    assert!(
        messages[0].image_blocks.is_empty(),
        "demotion must release image data so the budget accounting holds"
    );
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
