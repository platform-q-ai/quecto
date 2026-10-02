//! #1046: the stub format, the emergency ladder and creation-time message
//! spilling. Split from `context_pruning.rs` to
//! respect the 750-line source cap.

use std::sync::{Arc, Mutex};
use std::{future::Future, pin::Pin};

use super::messages::*;
use super::*;
use crate::application::sessions::ports::ContextSpillStore;
use crate::application::sessions::ports::SpillIndexList;
use crate::domain::message::{Message, Role};
use crate::domain::session::{SpillEntry, SpillIndex};
use crate::domain::session_identity::{SessionIdentity, SpillId};

fn id(k: &str) -> SessionIdentity {
    SessionIdentity::from_persisted_key(k)
}

/// The narrow handles the policy consumes, composed over the test store
/// exactly as the runtime composes them (D9 #1978).
fn retention(store: &Arc<MemStore>) -> crate::application::context::ContextRetention {
    crate::composition::retention::context_retention_over(store.clone())
}

/// Minimal in-memory spill store for the creation-time spill path.
#[derive(Debug, Default)]
struct MemStore {
    entries: Mutex<Vec<SpillEntry>>,
}

impl ContextSpillStore for MemStore {
    fn append(
        &self,
        _session_key: &SessionIdentity,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        self.entries.lock().unwrap().push(entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        _session_key: &SessionIdentity,
        id: &SpillId,
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
            .find(|e| e.id == id.as_str())
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn list_entries(&self, _session_key: &SessionIdentity) -> SpillIndexList<'_> {
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
        _session_key: &SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        self.entries.lock().unwrap().clear();
        Box::pin(async { Ok(()) })
    }
}

/// An old (previous-prompt) conversation message on `turn`, already spilled at
/// creation (spill_id stamped), as production guarantees after #1046 AC1.
fn conv_msg(role: Role, turn: u32, i: u32) -> Message {
    let content = format!("conversation message {i} {}", "padding ".repeat(20));
    let mut m = match role {
        Role::Assistant => Message::assistant(&content, vec![]),
        _ => Message::user(&content),
    };
    m.turn = Some(turn);
    m.spill_id = Some(format!("turn{turn}:msg:{}", role.as_str()));
    m
}

/// A session of `n` old conversation messages (alternating user/assistant on
/// distinct old turns) followed by the in-flight (turn-less) user prompt.
fn session_with_old_conv_messages(n: u32) -> Vec<Message> {
    let mut messages: Vec<Message> = (1..=n)
        .map(|i| {
            let role = if i % 2 == 0 {
                Role::Assistant
            } else {
                Role::User
            };
            conv_msg(role, i, i)
        })
        .collect();
    messages.push(Message::user("current question"));
    messages
}

// --- stub format (AC2) ---

#[test]
fn message_stub_contains_role_preview_tokens_and_recall_id() {
    let stub = message_collapse_stub(
        "assistant",
        "I analysed the codebase and",
        840,
        "turn12:msg:assistant",
    );
    assert!(
        stub.contains("[assistant:"),
        "stub must name the role: {stub}"
    );
    assert!(
        stub.contains("I analysed the codebase"),
        "stub must carry a preview: {stub}"
    );
    assert!(
        stub.contains("840 tokens"),
        "stub must carry tokens: {stub}"
    );
    assert!(
        stub.contains("recall(\"turn12:msg:assistant\")"),
        "stub must carry the recall id: {stub}"
    );
    assert!(!stub.contains('\n'), "stub must be a one-liner: {stub:?}");
}

// --- demotion ladder (AC6) ---

/// Old conversation messages with spill ids (already on disk) + prompt.
fn spillable_session(n: u32) -> Vec<Message> {
    session_with_old_conv_messages(n)
}

#[test]
fn ladder_collapses_to_stubs_before_dropping_anything() {
    // ~40 tokens/message stubbed vs ~45 full; budget met by stubbing alone.
    let mut messages = spillable_session(4);
    let full_total = estimate_total_tokens(&messages);
    let budget = full_total - 20; // slightly over budget: stubs suffice
    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 0);
    assert!(
        outcome.collapsed_to_stubs >= 1,
        "the first rung must demote full messages to stubs"
    );
    assert_eq!(
        outcome.dropped, 0,
        "nothing may be hard-dropped while stubbing suffices"
    );
    assert!(
        estimate_total_tokens(&messages) <= budget,
        "the budget must be met"
    );
    assert!(
        messages[0].is_collapsed && messages[0].content.contains("recall("),
        "oldest-first: the oldest message becomes a recall stub"
    );
    assert!(
        !outcome.over_budget,
        "control: a met budget must not be reported as unmet"
    );
}

#[test]
fn ladder_drops_stubs_only_when_stubbing_is_insufficient() {
    let mut messages = spillable_session(4);
    // Budget so tight that even all-stubs exceeds it: the second rung must
    // remove stubs entirely (content already on disk — manifest-only).
    let before = messages.len();
    let outcome = enforce_context_ceiling_ladder(&mut messages, 5, 0);
    assert!(
        outcome.dropped >= 1,
        "still-over-budget after stubbing must drop stubs entirely"
    );
    assert!(messages.len() < before, "dropped stubs leave the context");
    assert!(
        messages
            .iter()
            .filter(|m| m.role != Role::System && m.turn.is_some())
            .all(|m| m.is_collapsed),
        "no full un-collapsed old message may survive while stubs were dropped"
    );
}

#[test]
fn ladder_never_demotes_pinned_or_exempt_and_reports_unmet_budget() {
    let mut manifest = Message::system("[Session memory: 1 spilled entries via recall()]");
    manifest.is_pinned = true;
    manifest.is_manifest = true;
    let mut messages = vec![
        Message::system("system prompt"),
        manifest,
        Message::user("current question that is quite long indeed"),
    ];
    // Budget of 1 token: unmeetable — the pinned set alone exceeds it.
    let outcome = enforce_context_ceiling_ladder(&mut messages, 1, 2);
    assert_eq!(messages.len(), 3, "pinned/exempt messages must all survive");
    assert!(
        messages.iter().all(|m| !m.is_collapsed),
        "pinned/exempt content is never demoted at any rung"
    );
    assert!(
        outcome.over_budget,
        "an unmeetable ceiling must be reported so callers can warn/audit (#1044)"
    );
}

// --- creation-time spilling (AC1) ---

#[tokio::test]
async fn spill_conversation_message_appends_full_content_and_stamps_id() {
    let store = Arc::new(MemStore::default());
    let mut msg = Message::assistant("the full assistant reply text", vec![]);
    msg.turn = Some(3);
    spill_conversation_message(&mut msg, &retention(&store).retain, &id("s")).await;
    assert_eq!(
        msg.spill_id.as_deref(),
        Some("turn3:msg:assistant"),
        "the message must be stamped with its spill id at creation"
    );
    let entry = store
        .recall(&id("s"), &SpillId::new("turn3:msg:assistant"))
        .await
        .unwrap()
        .expect("the message must be recallable immediately after creation");
    assert_eq!(entry.content, "the full assistant reply text");
    assert_eq!(entry.tool, "assistant", "spills carry the role");
    assert_eq!(
        msg.content, "the full assistant reply text",
        "spilling must not disturb the in-context content"
    );
}

#[tokio::test]
async fn spill_conversation_message_dedups_ids_across_prompts() {
    // Turn numbering restarts each prompt: two turn-1 assistant replies in one
    // session must get distinct, individually recallable ids.
    let store = Arc::new(MemStore::default());
    let mut first = Message::assistant("prompt A reply", vec![]);
    first.turn = Some(1);
    spill_conversation_message(&mut first, &retention(&store).retain, &id("s")).await;
    let mut second = Message::assistant("prompt B reply", vec![]);
    second.turn = Some(1);
    spill_conversation_message(&mut second, &retention(&store).retain, &id("s")).await;
    assert_eq!(second.spill_id.as_deref(), Some("turn1:msg:assistant:2"));
    let entry = store
        .recall(&id("s"), &SpillId::new("turn1:msg:assistant:2"))
        .await
        .unwrap()
        .expect("the deduplicated id must be recallable");
    assert_eq!(entry.content, "prompt B reply");
}

// --- live counting on re-entry (AC2: stubs are not "live") ---

// --- ladder boundary: at/under budget is a no-op (AC6) ---

#[test]
fn ladder_is_a_no_op_at_or_under_budget() {
    let mut messages = spillable_session(4);
    let full_total = estimate_total_tokens(&messages);
    let before: Vec<String> = messages.iter().map(|m| m.content.clone()).collect();
    let outcome = enforce_context_ceiling_ladder(&mut messages, full_total, 0);
    assert_eq!(outcome.collapsed_to_stubs, 0, "at budget nothing may stub");
    assert_eq!(outcome.dropped, 0, "at budget nothing may drop");
    assert!(!outcome.over_budget);
    let after: Vec<String> = messages.iter().map(|m| m.content.clone()).collect();
    assert_eq!(before, after, "messages must be untouched at budget");
}

// --- pin_recent_turns tail on the current run's turn-stamped messages (AC3) ---

// --- ephemeral sessions (empty key): creation spilling must still persist ---
// PR #1048 follow-up: the empty-key guard in the conversation spill writer is
// removed so ephemeral runs match tool spilling (deliberately unguarded, see
// the NOTE in agent_loop_spill.rs) — collapse/ladder recall() stubs must stay
// resolvable in `--no-session` runs instead of pointing at nothing.

#[tokio::test]
async fn spill_conversation_message_persists_for_ephemeral_sessions() {
    let store = Arc::new(MemStore::default());
    let mut msg = Message::assistant("ephemeral reply text", vec![]);
    msg.turn = Some(1);
    let written = spill_conversation_message(&mut msg, &retention(&store).retain, &id("")).await;
    assert!(
        written,
        "an ephemeral (empty-key) session must still spill conversation \
         messages so later collapse stubs are recallable"
    );
    assert_eq!(
        msg.spill_id.as_deref(),
        Some("turn1:msg:assistant"),
        "the spill id must be stamped for ephemeral sessions too"
    );
    let entry = store
        .recall(&id(""), &SpillId::new("turn1:msg:assistant"))
        .await
        .unwrap()
        .expect("the ephemeral spill entry must be recallable");
    assert_eq!(entry.content, "ephemeral reply text");
}

// --- ladder rung 1: tiny messages whose stub is not cheaper are skipped ---

#[test]
fn ladder_skips_tiny_messages_whose_stub_would_not_be_cheaper() {
    // A tiny old message (stub estimate >= content estimate) among large ones.
    let mut tiny = Message::user("ok");
    tiny.turn = Some(1);
    tiny.spill_id = Some("turn1:msg:user".into());
    let mut messages = vec![tiny];
    for i in 2..=4u32 {
        messages.push(conv_msg(Role::Assistant, i, i));
    }
    messages.push(Message::user("current question"));
    let total = estimate_total_tokens(&messages);
    // Slightly over budget: stubbing the large messages suffices, so the
    // second rung never runs and the tiny message's fate is rung 1's alone.
    let budget = total - 20;
    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 0);
    assert!(
        outcome.collapsed_to_stubs >= 1,
        "positive control: large messages must be stubbed"
    );
    assert_eq!(outcome.dropped, 0, "budget must be met by stubbing alone");
    assert!(
        !messages[0].is_collapsed && messages[0].content == "ok",
        "a message whose stub would be no cheaper than its content must be \
         skipped at rung 1, not inflated into a larger stub; got: {}",
        messages[0].content
    );
    assert!(
        estimate_total_tokens(&messages) <= budget,
        "the budget must still be met"
    );
}

// --- ladder rung 2: drop order is oldest-first ---

#[test]
fn ladder_second_rung_drops_the_oldest_stub_first() {
    // All old messages are already stubs of equal size; the budget is met
    // after removing exactly ONE of them — the oldest must be the one gone.
    let mut messages = session_with_old_conv_messages(3);
    let n_old = 3;
    for m in messages[..n_old].iter_mut() {
        let tokens = estimate_tokens(&m.content);
        let id = m.spill_id.clone().unwrap();
        m.content = message_collapse_stub(m.role.as_str(), &m.content, tokens, &id);
        m.is_collapsed = true;
        m.invalidate_token_cache();
    }
    let total = estimate_total_tokens(&messages);
    let one_stub = estimate_message_tokens(&messages[0]);
    // Removing one stub meets the budget; removing two would be over-pruning.
    let budget = total - 1;
    assert!(
        one_stub > 1,
        "sanity: a stub removal must free enough tokens"
    );
    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 0);
    assert_eq!(
        outcome.dropped, 1,
        "exactly one stub removal suffices for this budget"
    );
    let contents: Vec<&str> = messages.iter().map(|m| m.content.as_str()).collect();
    assert!(
        !contents.iter().any(|c| c.contains("turn1:msg:user")),
        "the OLDEST stub (turn1) must be the one removed, got: {contents:?}"
    );
    assert!(
        contents.iter().any(|c| c.contains("turn2:msg:assistant"))
            && contents.iter().any(|c| c.contains("turn3:msg:user")),
        "newer stubs must survive a partial drop, got: {contents:?}"
    );
}

// --- creation-spill id dedup: third collision mints :3, not a re-used :2 ---

#[tokio::test]
async fn creation_spill_third_collision_mints_suffix_3() {
    let store = Arc::new(MemStore::default());
    for (i, text) in ["prompt A reply", "prompt B reply", "prompt C reply"]
        .iter()
        .enumerate()
    {
        let mut msg = Message::assistant(*text, vec![]);
        msg.turn = Some(1);
        spill_conversation_message(&mut msg, &retention(&store).retain, &id("s")).await;
        let expected = match i {
            0 => "turn1:msg:assistant".to_string(),
            n => format!("turn1:msg:assistant:{}", n + 1),
        };
        assert_eq!(
            msg.spill_id.as_deref(),
            Some(expected.as_str()),
            "collision {i} must mint the next free suffix"
        );
    }
    let entry = store
        .recall(&id("s"), &SpillId::new("turn1:msg:assistant:3"))
        .await
        .unwrap()
        .expect("the third-collision id must be recallable");
    assert_eq!(
        entry.content, "prompt C reply",
        "turn1:msg:assistant:3 must recall the THIRD colliding message"
    );
}
#[tokio::test]
async fn mem_store_default_has_entries_is_false() {
    assert!(!MemStore::default().has_entries(&id("s")).await.unwrap());
}

#[tokio::test]
async fn mem_store_trait_surface_clear_empties_entries() {
    let store = Arc::new(MemStore::default());
    let entry = SpillEntry {
        id: "id1".into(),
        tool: "bash".into(),
        input_preview: "echo".into(),
        tokens: 2,
        content: "out".into(),
    };
    store.append(&id("s"), &entry).await.unwrap();
    assert_eq!(store.list_entries(&id("s")).await.unwrap().len(), 1);
    store.clear(&id("s")).await.unwrap();
    assert!(store.list_entries(&id("s")).await.unwrap().is_empty());
}
