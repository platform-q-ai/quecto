use super::*;
use crate::application::context_pruning;
use crate::domain::message::Message;
use crate::domain::session::{ContextSpillStore, SpillEntry, SpillIndex};
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

fn manager(max_context_tokens: usize) -> ContextManager {
    ContextManager::new(ContextManagerConfig {
        spill_store: Some(Arc::new(MemSpillStore::default())),
        session_key: "test-session".to_string(),
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
        .prepare_provider_context(&mut messages, 1, false)
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
        .prepare_provider_context(&mut messages, 1, true)
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
