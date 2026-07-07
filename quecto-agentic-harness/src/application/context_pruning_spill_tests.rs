//! #951: the spilling context ceiling — returns dropped messages for
//! spilling, tail-pins recent turns and the in-flight user prompt, and never
//! drops pinned messages (system prompt, manifest).

use super::*;
use crate::domain::message::{Message, Role};

fn assistant_on_turn(content: &str, turn: u32) -> Message {
    let mut m = Message::assistant(content, vec![]);
    m.turn = Some(turn);
    m
}

#[test]
fn ceiling_spilling_returns_dropped_messages_for_spilling() {
    let big = "x".repeat(600); // ~150 tokens
    let mut messages = vec![
        assistant_on_turn(&big, 1),
        assistant_on_turn(&big, 2),
        Message::user("current prompt"),
    ];
    // Budget 200: only the oldest assistant turn must go — and it must be
    // returned to the caller for spilling, not silently lost.
    let dropped = enforce_context_ceiling_spilling(&mut messages, 200, 1);
    assert_eq!(
        dropped.len(),
        1,
        "the dropped assistant turn must be returned for spilling"
    );
    assert_eq!(dropped[0].content, big);
    assert_eq!(dropped[0].turn, Some(1));
    assert_eq!(
        messages.iter().filter(|m| m.content == big).count(),
        1,
        "exactly one big assistant turn should remain in context"
    );
}

#[test]
fn ceiling_spilling_under_budget_drops_and_spills_nothing() {
    let mut messages = vec![
        assistant_on_turn("short answer", 1),
        Message::user("current prompt"),
    ];
    let before = messages.len();
    // Total is well under budget: no drops, nothing returned for spilling.
    let dropped = enforce_context_ceiling_spilling(&mut messages, 1000, 2);
    assert!(
        dropped.is_empty(),
        "a history within budget must produce no spurious spills"
    );
    assert_eq!(messages.len(), before, "no message may be dropped");
}

#[test]
fn ceiling_spilling_never_drops_most_recent_turns() {
    let big = "x".repeat(600); // ~150 tokens each
    let mut messages: Vec<Message> = (1..=4).map(|t| assistant_on_turn(&big, t)).collect();
    // Budget impossible to meet — the 2 most recent turns must survive anyway.
    enforce_context_ceiling_spilling(&mut messages, 10, 2);
    for turn in [3u32, 4u32] {
        assert!(
            messages.iter().any(|m| m.turn == Some(turn)),
            "turn {turn} is within the pinned tail and must never be dropped"
        );
    }
    assert!(
        messages
            .iter()
            .all(|m| m.turn != Some(1) && m.turn != Some(2)),
        "turns outside the pinned tail must still be dropped to approach budget"
    );
}

#[test]
fn ceiling_spilling_pins_trailing_turnless_user_prompt() {
    let big = "x".repeat(600);
    // An earlier user prompt comes first so the prompt under test is trailing
    // but NOT the first user message — a "pin the first user message only"
    // implementation must fail this test.
    let mut old_user = Message::user("old prompt");
    old_user.turn = Some(1);
    let mut messages = vec![
        old_user,
        assistant_on_turn(&big, 1),
        Message::user("what next?"),
    ];
    // Budget of 1 token: even so the in-flight user prompt (no turn yet)
    // must never be dropped.
    enforce_context_ceiling_spilling(&mut messages, 1, 1);
    assert!(
        messages
            .iter()
            .any(|m| m.role == Role::User && m.content == "what next?"),
        "trailing turn-less user prompt must survive the ceiling"
    );
    assert!(
        !messages.iter().any(|m| m.content == "old prompt"),
        "the earlier user prompt is droppable and must go to approach budget"
    );
}

#[test]
fn ceiling_spilling_never_drops_system_prompt_or_manifest() {
    let big = "x".repeat(600);
    let mut manifest = Message::system("[Session memory: 1 spilled entry via recall()]");
    manifest.is_pinned = true;
    manifest.is_manifest = true;
    let mut messages = vec![
        Message::system("system prompt"),
        manifest,
        assistant_on_turn(&big, 1),
        assistant_on_turn(&big, 2),
        assistant_on_turn(&big, 3),
    ];
    // Budget impossible to meet: pinned messages must survive AND must never
    // leak into the returned (to-be-spilled) set.
    let dropped = enforce_context_ceiling_spilling(&mut messages, 1, 1);
    assert!(
        messages
            .iter()
            .any(|m| m.role == Role::System && !m.is_manifest),
        "system prompt must never be dropped"
    );
    assert!(
        messages.iter().any(|m| m.is_manifest),
        "spill manifest must never be dropped"
    );
    assert!(
        dropped
            .iter()
            .all(|m| !m.is_pinned && m.role != Role::System),
        "pinned messages must never appear in the spill set"
    );
    assert!(
        !dropped.is_empty(),
        "positive control: unpinned old turns must still be dropped"
    );
}

#[test]
fn manifest_text_distinguishes_tool_and_message_spills() {
    let entries = vec![
        SpillIndex {
            id: "turn1:bash:0".to_string(),
            tool: "bash".to_string(),
            input_preview: "echo hello".to_string(),
            tokens: 100,
        },
        SpillIndex {
            id: "turn2:msg:assistant".to_string(),
            tool: "assistant".to_string(),
            input_preview: "analysis of the build failure".to_string(),
            tokens: 250,
        },
    ];
    let text = build_manifest_text(&entries);
    assert!(
        text.contains("turn1:bash:0"),
        "manifest must keep the tool-spill id form; got: {text}"
    );
    assert!(
        text.contains("turn2:msg:assistant"),
        "manifest must render the message-spill id form; got: {text}"
    );
}
