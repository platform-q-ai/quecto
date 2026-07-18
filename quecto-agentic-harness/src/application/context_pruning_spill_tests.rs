//! #951/#1046: the demotion-ladder context ceiling — tail-pins recent turns
//! and the in-flight user prompt, and never demotes pinned messages (system
//! prompt, manifest). Content is spilled at creation, so the ladder can drop
//! stubs without a caller-side spill step.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::*;
use crate::domain::message::{Message, Role};
use crate::domain::session::{SpillEntry, SpillIndex};

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
fn ceiling_ladder_under_budget_demotes_nothing() {
    let mut messages = vec![
        assistant_on_turn("short answer", 1),
        Message::user("current prompt"),
    ];
    let before = messages.len();
    // Total is well under budget: no demotion at any rung.
    let outcome = messages::enforce_context_ceiling_ladder(&mut messages, 1000, 2);
    assert_eq!(outcome.collapsed_to_stubs, 0, "nothing may stub at budget");
    assert_eq!(outcome.dropped, 0, "nothing may drop at budget");
    assert_eq!(messages.len(), before, "no message may be removed");
    assert!(messages.iter().all(|m| !m.is_collapsed));
}

#[test]
fn ceiling_ladder_never_demotes_most_recent_turns() {
    let big = "x".repeat(600); // ~150 tokens each
    let mut messages: Vec<Message> = (1..=4).map(|t| assistant_on_turn(&big, t)).collect();
    // Budget impossible to meet — the 2 most recent turns must survive in
    // full anyway; older turns stub and then drop.
    messages::enforce_context_ceiling_ladder(&mut messages, 10, 2);
    for turn in [3u32, 4u32] {
        let msg = messages
            .iter()
            .find(|m| m.turn == Some(turn))
            .unwrap_or_else(|| panic!("turn {turn} is tail-pinned and must remain"));
        assert!(
            !msg.is_collapsed,
            "turn {turn} is within the pinned tail and must never be demoted"
        );
    }
    assert!(
        messages
            .iter()
            .all(|m| m.turn != Some(1) && m.turn != Some(2)),
        "turns outside the pinned tail must be dropped to approach budget"
    );
}

#[test]
fn ceiling_ladder_pins_trailing_turnless_user_prompt() {
    let big = "x".repeat(600);
    // An earlier user prompt comes first so the prompt under test is trailing
    // but NOT the first user message — a "pin the first user message only"
    // implementation must fail this test.
    let mut old_user = Message::user("old prompt");
    old_user.turn = Some(7); // stamped: belongs to an earlier prompt's turn
    let mut messages = vec![
        old_user,
        assistant_on_turn(&big, 7),
        Message::user("what next?"),
    ];
    // Budget of 1 token: even so the in-flight user prompt (no turn yet)
    // must never be demoted. The ladder's tail fallback protects the previous
    // prompt's single most recent turn (7) as stubs, so with pin 0 here the
    // old turn is fully droppable.
    let outcome = messages::enforce_context_ceiling_ladder(&mut messages, 1, 0);
    let prompt = messages
        .iter()
        .find(|m| m.role == Role::User && m.content == "what next?")
        .expect("trailing turn-less user prompt must survive the ceiling");
    assert!(
        !prompt.is_collapsed,
        "the in-flight prompt is never demoted"
    );
    assert!(
        !messages.iter().any(|m| m.content == "old prompt"),
        "the earlier user prompt is droppable and must go to approach budget"
    );
    assert!(
        outcome.over_budget,
        "a 1-token budget with a surviving prompt is reported unmet"
    );
}

#[test]
fn ceiling_ladder_tail_fallback_protects_previous_prompt_turns() {
    // A new prompt was just submitted (no turns stamped yet): the tail
    // fallback keeps pin_recent_turns protecting the PREVIOUS prompt's most
    // recent turns instead of leaving everything droppable (#1045).
    let big = "x".repeat(600);
    let mut old_prompt = Message::user("old question");
    old_prompt.turn = None; // previous prompt boundary
    let mut messages = vec![
        old_prompt,
        assistant_on_turn(&big, 1),
        assistant_on_turn(&big, 2),
        Message::user("new question just submitted"),
    ];
    messages::enforce_context_ceiling_ladder(&mut messages, 10, 2);
    for turn in [1u32, 2u32] {
        let msg = messages
            .iter()
            .find(|m| m.turn == Some(turn))
            .unwrap_or_else(|| panic!("previous prompt's turn {turn} must stay tail-pinned"));
        assert!(!msg.is_collapsed, "tail-pinned turn {turn} stays full");
    }
}

#[test]
fn ceiling_ladder_never_demotes_system_prompt_or_manifest() {
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
    // Budget impossible to meet: pinned messages must survive un-demoted.
    let outcome = messages::enforce_context_ceiling_ladder(&mut messages, 1, 1);
    let system = messages
        .iter()
        .find(|m| m.role == Role::System && !m.is_manifest)
        .expect("system prompt must never be dropped");
    assert!(!system.is_collapsed, "system prompt is never demoted");
    let manifest = messages
        .iter()
        .find(|m| m.is_manifest)
        .expect("spill manifest must never be dropped");
    assert!(!manifest.is_collapsed, "manifest is never demoted");
    assert!(
        outcome.dropped > 0,
        "positive control: unpinned old turns must still be dropped"
    );
}

#[tokio::test]
async fn manifest_text_stays_static_across_tool_and_message_spills() {
    // Exercise real tool/message spill IDs, then verify none of their dynamic
    // bytes enter the front-positioned cache prefix (#1118).
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
    assert_eq!(
        entries.len(),
        2,
        "test setup should create both spill kinds"
    );

    let mut messages = vec![Message::system("system prompt"), Message::user("prompt")];
    assert!(update_spill_manifest(&mut messages, &store, "s").await);
    let manifest = messages
        .iter()
        .find(|message| message.is_manifest)
        .expect("a populated spill store should produce guidance");
    assert_eq!(manifest.content, build_manifest_text());
    assert!(!manifest.content.contains("turn1:bash:0"));
    assert!(!manifest.content.contains("turn1:msg:assistant"));
    assert!(!manifest.content.contains("echo hello"));
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
    let outcome = messages::enforce_context_ceiling_ladder(&mut messages, 200, 2);
    assert!(
        outcome.collapsed_to_stubs + outcome.dropped > 0,
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
    messages::enforce_context_ceiling_ladder(&mut messages, 10, 2);
    assert!(
        messages
            .iter()
            .any(|m| m.content == "real question" && !m.is_collapsed),
        "the in-flight prompt must never lose its protection to feedback"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.turn == Some(2) && !m.is_collapsed),
        "current-prompt recent turns must stay tail-pinned"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.turn == Some(3) && !m.is_collapsed),
        "the stamped feedback message is a recent turn and stays pinned"
    );
    assert!(
        !messages.iter().any(|m| m.turn == Some(7)),
        "the earlier prompt's turn must still be droppable"
    );
}
