use super::*;
use crate::application::context_pruning;
use crate::application::sessions::ports::ContextSpillStore;
use crate::domain::message::Message;
use crate::domain::session::{SpillEntry, SpillIndex};
use crate::domain::session_identity::{SessionIdentity, SpillId};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct MemSpillStore {
    entries: Mutex<Vec<SpillEntry>>,
}

impl ContextSpillStore for MemSpillStore {
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

    fn list_entries(
        &self,
        _session_key: &SessionIdentity,
    ) -> crate::application::sessions::ports::SpillIndexList<'_> {
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

fn manager(max_context_tokens: usize) -> ContextManager {
    ContextManager::new(ContextManagerConfig {
        retention: Some(crate::composition::retention::context_retention_over(
            Arc::new(MemSpillStore::default()),
        )),
        session_key: SessionIdentity::from_persisted_key("test-session"),
        context_collapse_after_tool_calls: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens,
        pin_recent_turns: 2,
        context_collapse_after_messages: context_pruning::COLLAPSE_DISABLED,
        model_context_window: None,
    })
}

fn long_message(turn: u32) -> Message {
    let mut msg = Message::assistant("x".repeat(2_000), vec![]);
    msg.turn = Some(turn);
    msg.spill_id = Some(format!("turn{turn}:msg:assistant"));
    msg
}

#[tokio::test]
async fn context_manager_plan_preserves_pinned_recent_turns() {
    let manager = manager(10);
    let mut messages = vec![
        long_message(1),
        long_message(2),
        long_message(3),
        long_message(4),
        Message::user("current prompt"),
    ];

    let plan = manager
        .prepare_provider_context(&mut messages, manager.effective_max_context_tokens(), false)
        .await;

    assert!(
        messages.iter().any(|m| m.turn == Some(3)),
        "turn 3 should be pinned as one of the two most recent completed turns"
    );
    assert!(
        messages.iter().any(|m| m.turn == Some(4)),
        "turn 4 should be pinned as one of the two most recent completed turns"
    );
    assert!(
        !messages.iter().any(|m| matches!(m.turn, Some(1 | 2))),
        "older turns should be dropped through the context-manager plan"
    );
    assert!(
        plan.durable_prefix_dirty,
        "dropping older persisted messages must request durable prefix reconciliation"
    );
}

#[tokio::test]
async fn context_manager_marks_dirty_when_manifest_layout_shifts() {
    let manager = manager(190_000);
    let mut manifest = Message::system(context_pruning::build_manifest_text());
    manifest.is_pinned = true;
    manifest.is_manifest = true;
    let mut messages = vec![manifest];

    let plan = manager
        .prepare_provider_context(&mut messages, manager.effective_max_context_tokens(), true)
        .await;

    assert!(
        messages.iter().all(|m| !m.is_manifest),
        "empty spill store should remove the persisted manifest"
    );
    assert!(
        plan.durable_prefix_dirty,
        "manifest insertion/removal shifts persisted positions and must be dirty"
    );
}

#[test]
fn context_manager_reconciles_provider_truth_across_local_estimate_changes() {
    let manager = manager(190_000);

    manager.observe_provider_context_gauge(1_000, 100);

    assert_eq!(
        manager.reconcile_context_gauge(80),
        980,
        "provider truth should be carried forward by the local estimate delta"
    );
}

#[test]
fn context_manager_is_the_agent_loop_context_boundary() {
    let manager = manager(190_000);

    assert_eq!(manager.effective_max_context_tokens(), 190_000);
    assert_eq!(
        manager.context_knob_snapshot(),
        (2, context_pruning::COLLAPSE_DISABLED)
    );
    let mut msg = Message::assistant("spill me", vec![]);
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(manager.spill_conversation_message(&mut msg));
    assert!(msg.spill_id.is_some());
}

#[path = "context_gauge_tests.rs"]
mod gauge_tests;

/// #2160: tokens sent with every request outside the messages (the tool
/// definitions) shrink the room the messages have.
#[tokio::test]
async fn tokens_sent_with_every_request_shrink_the_message_budget() {
    let conversation = || {
        vec![
            long_message(1),
            long_message(2),
            long_message(3),
            long_message(4),
            Message::user("now"),
        ]
    };
    let total = context_pruning::estimate_total_tokens(&conversation());
    let manager = manager(total + 10);

    let mut roomy = conversation();
    manager
        .prepare_provider_context(&mut roomy, total + 10, false)
        .await;
    assert!(roomy.iter().any(|m| m.turn == Some(1)), "everything fits");

    let mut tight = conversation();
    let budget = total + 10 - total / 2;
    let plan = manager
        .prepare_provider_context(&mut tight, budget, false)
        .await;
    assert!(
        !tight.iter().any(|m| m.turn == Some(1)),
        "the oldest turn makes room for what every request carries"
    );
    // The ladder holds the messages to the budget (the spill manifest is
    // added after it and is not part of this bound).
    let messages: Vec<Message> = tight.iter().filter(|m| !m.is_manifest).cloned().collect();
    assert!(!plan.over_budget);
    assert!(context_pruning::estimate_total_tokens(&messages) <= budget);
}

/// #2213: once the ceiling is crossed the plan prunes down to the low-water
/// mark, so the next over-ceiling append lands in the headroom and the
/// durable prefix is rewritten once, not on every turn.
#[tokio::test]
async fn two_consecutive_over_ceiling_appends_latch_the_prefix_dirty_once() {
    let manager = manager(190_000);
    let mut messages = vec![Message::user("current prompt")];
    messages.extend((1..=8).map(long_message));
    let per_turn = context_pruning::estimate_total_tokens(&[long_message(9)]);
    let budget = context_pruning::estimate_total_tokens(&messages) + per_turn / 2;

    let mut latches = 0;
    for turn in [9, 10] {
        messages.push(long_message(turn));
        let plan = manager
            .prepare_provider_context(&mut messages, budget, false)
            .await;
        assert!(!plan.over_budget);
        latches += usize::from(plan.durable_prefix_dirty);
    }
    assert_eq!(latches, 1, "one prefix rewrite per batch, not per turn");
}
