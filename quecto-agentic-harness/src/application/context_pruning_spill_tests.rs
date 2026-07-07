//! #951: the spilling context ceiling — returns dropped messages for
//! spilling, tail-pins recent turns and the in-flight user prompt, and never
//! drops pinned messages (system prompt, manifest).

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::*;
use crate::domain::message::{Message, Role};
use crate::domain::session::SpillEntry;

fn assistant_on_turn(content: &str, turn: u32) -> Message {
    let mut m = Message::assistant(content, vec![]);
    m.turn = Some(turn);
    m
}

/// Minimal in-memory spill store for exercising the real spill-to-store path.
#[derive(Debug, Default)]
struct MemStore {
    entries: Mutex<Vec<SpillEntry>>,
}

impl ContextSpillStore for MemStore {
    fn append(
        &self,
        _session_key: &str,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        self.entries.lock().unwrap().push(entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        _session_key: &str,
        id: &str,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<SpillEntry>, crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        let found = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.id == id)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn list_entries(&self, _session_key: &str) -> crate::domain::session::SpillIndexList<'_> {
        let index: Vec<SpillIndex> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .map(|e| SpillIndex {
                id: e.id.clone(),
                tool: e.tool.clone(),
                input_preview: e.input_preview.clone(),
                tokens: e.tokens,
            })
            .collect();
        Box::pin(async move { Ok(Arc::new(index)) })
    }

    fn clear(
        &self,
        _session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        self.entries.lock().unwrap().clear();
        Box::pin(async { Ok(()) })
    }
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

#[tokio::test]
async fn manifest_text_distinguishes_tool_and_message_spills() {
    // Exercise the REAL id construction: a tool spill already in the store,
    // plus an assistant turn filed by the creation-time spill writer (#1046).
    let store = MemStore::default();
    store
        .append(
            "s",
            &SpillEntry {
                id: "turn1:bash:0".to_string(),
                tool: "bash".to_string(),
                input_preview: "echo hello".to_string(),
                tokens: 100,
                content: "hello".to_string(),
            },
        )
        .await
        .unwrap();
    let big = "x".repeat(600);
    let mut assistant = assistant_on_turn(&big, 1);
    messages::spill_conversation_message(&mut assistant, &store, "s").await;
    let entries = store.list_entries("s").await.unwrap();
    let text = build_manifest_text(&entries);
    assert!(
        text.contains("turn1:bash:0"),
        "manifest must keep the tool-spill id form; got: {text}"
    );
    assert!(
        text.contains("turn1:msg:assistant"),
        "manifest must render the message-spill id produced by the ceiling; got: {text}"
    );
}

// --- review fixes for PR #1043 ---

#[tokio::test]
async fn message_spill_ids_never_collide_across_prompts() {
    // Turn numbering restarts each prompt, so two different prompts can both
    // file a "turn 1" assistant reply into the same session-persistent store.
    // Every spill must stay individually recallable.
    let store = MemStore::default();
    for content in ["prompt A reply", "prompt B reply"] {
        let mut assistant = assistant_on_turn(&content.repeat(50), 1);
        messages::spill_conversation_message(&mut assistant, &store, "s").await;
    }
    let entries = store.list_entries("s").await.unwrap();
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids.len(), 2, "both replies must spill");
    assert_eq!(ids[0], "turn1:msg:assistant");
    assert_eq!(
        ids[1], "turn1:msg:assistant:2",
        "a colliding base id must be de-duplicated, got: {ids:?}"
    );
    let second = store
        .recall("s", "turn1:msg:assistant:2")
        .await
        .unwrap()
        .expect("the second prompt's spill must be recallable under its own id");
    assert!(second.content.contains("prompt B reply"));
}

#[tokio::test]
async fn turnless_user_spills_get_distinct_ids() {
    // Production never turn-stamps user prompts; several spilled past prompts
    // must not all collide on `turn0:msg:user`.
    let store = MemStore::default();
    let big = "z".repeat(600);
    for _ in 0..2 {
        let mut user = Message::user(&big);
        messages::spill_conversation_message(&mut user, &store, "s").await;
    }
    let entries = store.list_entries("s").await.unwrap();
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1], "turn-less user spills must not collide");
}

#[test]
fn ceiling_still_prunes_fully_turnless_history() {
    // A chat-only session saved by a pre-turn-stamping build has no turn on
    // any message. The ceiling must still be enforced on resume — only the
    // in-flight prompt (last turn-less user message) is protected.
    let big = "x".repeat(600); // ~150 tokens each
    let mut messages = vec![
        Message::user(&big),
        Message::assistant(&big, vec![]),
        Message::user(&big),
        Message::assistant(&big, vec![]),
        Message::user("current prompt"),
    ];
    let dropped = enforce_context_ceiling_spilling(&mut messages, 200, 2);
    assert!(
        !dropped.is_empty(),
        "a fully turn-less over-budget history must still be pruned"
    );
    assert!(
        estimate_total_tokens(&messages) <= 200,
        "budget must be met on resumed turn-less histories"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.role == Role::User && m.content == "current prompt"),
        "the in-flight prompt must survive"
    );
}

#[test]
fn stamped_trailing_user_feedback_does_not_usurp_prompt_boundary() {
    // Mid-prompt malformed-feedback user messages are turn-stamped (see
    // append_malformed_feedback), so the real in-flight prompt keeps its
    // boundary role and this prompt's recent turns keep tail-pin protection.
    let big = "x".repeat(600);
    let mut feedback = Message::user("your request was malformed, fix it");
    feedback.turn = Some(3);
    let mut messages = vec![
        assistant_on_turn(&big, 7), // earlier prompt's turn — droppable
        Message::user("real question"),
        assistant_on_turn(&big, 2),
        feedback,
    ];
    enforce_context_ceiling_spilling(&mut messages, 10, 2);
    assert!(
        messages.iter().any(|m| m.content == "real question"),
        "the in-flight prompt must never lose its protection to feedback"
    );
    assert!(
        messages.iter().any(|m| m.turn == Some(2)),
        "current-prompt recent turns must stay tail-pinned"
    );
    assert!(
        messages.iter().any(|m| m.turn == Some(3)),
        "the stamped feedback message is a recent turn and stays pinned"
    );
    assert!(
        !messages.iter().any(|m| m.turn == Some(7)),
        "the earlier prompt's turn must still be droppable"
    );
}
