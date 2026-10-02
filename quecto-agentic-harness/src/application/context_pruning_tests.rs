use super::messages::{message_collapse_stub, message_stub_without_recall};
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
    // 1 text + 75 image + 3 for tool_call_id "call_1" ("call_" prose 2,
    // "1" dense 1: #2212)
    assert_eq!(estimate_message_tokens(&msg), 79);
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

// --- #1017: collapse triggers on number of tool calls, default 50 ---

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

// --- rewind stub stripping: rfind, not find (PR #1048 round-2 review) ---

#[test]
fn stub_without_recall_strips_only_the_trailing_clause() {
    // The 60-char preview is arbitrary user text and can itself contain the
    // " — recall(" marker (e.g. a user pasting a stub back into chat). Only
    // the formatter-appended trailing clause may be stripped; a first-match
    // implementation truncates inside the preview and corrupts the stub.
    let pasted = r#"[user: "x" (5 tokens) — recall("id")] please explain"#;
    let stub = message_collapse_stub("user", pasted, 13, "turn3:msg:user");
    let stripped = message_stub_without_recall(&stub);
    assert!(
        !stripped.contains("turn3:msg:user"),
        "the real trailing recall clause must be stripped, got: {stripped}"
    );
    assert!(
        stripped.contains(r#"recall("id")"#),
        "the preview text (including a quoted recall marker) must be intact, got: {stripped}"
    );
    assert!(
        stripped.contains("(13 tokens)"),
        "the token annotation must survive stripping, got: {stripped}"
    );
}

#[test]
fn stub_without_recall_is_identity_without_a_clause() {
    assert_eq!(message_stub_without_recall("plain text"), "plain text");
}
