use super::*;
use crate::infrastructure::tools::subagent_registry::{
    SequencedSubagentNotification, SubagentEntry, SubagentNotification, WorkflowSnapshot,
    new_registry,
};

fn poison_registry(registry: &SubagentRegistry) {
    let cloned = registry.clone();
    let _ = std::thread::spawn(move || {
        let _guard = cloned.lock().unwrap();
        panic!("poison registry for coverage");
    })
    .join();
    assert!(registry.lock().is_err(), "registry should be poisoned");
}

fn note(seq: u64) -> SequencedSubagentNotification {
    SequencedSubagentNotification::new(
        seq,
        SubagentNotification::Stalled {
            agent_id: "bot".into(),
            workflow_mode: "active".into(),
            steps_completed: 1,
            steps_total: 2,
        },
    )
}

#[test]
fn take_stalled_snapshot_consumes_latch_and_honors_terminal_guards() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new("/tmp/bot.sock".into(), 1);
    entry.workflow = Some(WorkflowSnapshot {
        mode: "active".into(),
        steps_completed: 1,
        steps_total: 3,
    });
    entry.stalled_armed = true;
    registry.lock().unwrap().insert("bot".into(), entry);

    let snap = take_stalled_snapshot(&registry, "bot").unwrap();
    assert_eq!(snap.steps_total, 3);
    assert!(take_stalled_snapshot(&registry, "bot").is_none());
    registry.lock().unwrap().get_mut("bot").unwrap().run_error = Some("boom".into());
    registry
        .lock()
        .unwrap()
        .get_mut("bot")
        .unwrap()
        .stalled_armed = true;
    assert!(take_stalled_snapshot(&registry, "bot").is_none());
}

#[test]
fn completion_armed_is_one_shot() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("bot".into(), SubagentEntry::new("/tmp/bot.sock".into(), 1));
    assert!(take_completion_armed(&registry, "bot"));
    assert!(!take_completion_armed(&registry, "bot"));
    assert!(!take_completion_armed(&registry, "missing"));
}

#[tokio::test]
async fn retry_pending_stalls_claims_and_resends_retained_alerts() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new("/tmp/bot.sock".into(), 1);
    entry.pending_stall = Some(note(7));
    registry.lock().unwrap().insert("bot".into(), entry);
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    retry_pending_stalls(&registry, Some(&tx));
    assert_eq!(rx.recv().await.unwrap().sequence, 7);
    assert!(registry.lock().unwrap()["bot"].pending_stall.is_none());
}

#[tokio::test]
async fn classify_workflow_idle_only_sends_exhausted_stalls() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new("/tmp/bot.sock".into(), 1);
    entry.workflow = Some(WorkflowSnapshot {
        mode: "selecting_template".into(),
        steps_completed: 0,
        steps_total: 0,
    });
    entry.stalled_armed = true;
    registry.lock().unwrap().insert("bot".into(), entry);
    let (tx, mut rx) = tokio::sync::mpsc::channel(2);
    classify_workflow_idle_stall(
        &registry,
        Some(&tx),
        "bot",
        1,
        &serde_json::json!({"reason":"completed"}),
    );
    assert!(rx.try_recv().is_err());
    classify_workflow_idle_stall(
        &registry,
        Some(&tx),
        "bot",
        2,
        &serde_json::json!({"reason":"exhausted"}),
    );
    assert_eq!(rx.recv().await.unwrap().sequence, 2);
}

#[tokio::test]
async fn stall_delivery_retains_on_full_channel_and_retry_no_tx_is_noop() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("bot".into(), SubagentEntry::new("/tmp/bot.sock".into(), 1));
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tx.try_send(note(1)).unwrap();

    deliver_or_retain_stall(&registry, &tx, "bot", note(2));
    assert_eq!(
        registry.lock().unwrap()["bot"]
            .pending_stall
            .as_ref()
            .unwrap()
            .sequence,
        2
    );
    retry_pending_stalls(&registry, None);
    assert!(registry.lock().unwrap()["bot"].pending_stall.is_some());
    assert_eq!(rx.recv().await.unwrap().sequence, 1);
}

#[test]
fn claim_pending_stall_rejects_mismatch_and_missing_agent() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new("/tmp/bot.sock".into(), 1);
    entry.pending_stall = Some(note(7));
    registry.lock().unwrap().insert("bot".into(), entry);

    assert!(!claim_pending_stall(&registry, "missing", &note(7)));
    assert!(!claim_pending_stall(&registry, "bot", &note(8)));
    assert_eq!(
        registry.lock().unwrap()["bot"]
            .pending_stall
            .as_ref()
            .unwrap()
            .sequence,
        7
    );
    assert!(claim_pending_stall(&registry, "bot", &note(7)));
    assert!(registry.lock().unwrap()["bot"].pending_stall.is_none());
}

#[test]
fn stall_helpers_recover_from_poisoned_registry_lock() {
    let registry = new_registry();
    let mut entry = SubagentEntry::new("/tmp/bot.sock".into(), 1);
    entry.workflow = Some(WorkflowSnapshot {
        mode: "active".into(),
        steps_completed: 1,
        steps_total: 2,
    });
    entry.stalled_armed = true;
    entry.pending_stall = Some(note(9));
    registry.lock().unwrap().insert("bot".into(), entry);
    poison_registry(&registry);

    assert!(!claim_pending_stall(&registry, "bot", &note(8)));
    assert!(take_stalled_snapshot(&registry, "bot").is_some());
    assert!(take_completion_armed(&registry, "bot"));
    retry_pending_stalls(&registry, None);
    assert!(
        registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key("bot")
    );
}

#[tokio::test]
async fn retain_pending_stall_without_runtime_and_closed_channel_are_safe() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("bot".into(), SubagentEntry::new("/tmp/bot.sock".into(), 1));
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    drop(rx);
    deliver_or_retain_stall(&registry, &tx, "missing", note(1));
    deliver_or_retain_stall(&registry, &tx, "bot", note(2));
    assert_eq!(
        registry.lock().unwrap()["bot"]
            .pending_stall
            .as_ref()
            .unwrap()
            .sequence,
        2
    );
}

#[test]
fn retain_pending_stall_from_sync_context_does_not_require_runtime() {
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("bot".into(), SubagentEntry::new("/tmp/bot.sock".into(), 1));
    let (tx, rx) = tokio::sync::mpsc::channel::<SequencedSubagentNotification>(1);
    drop(rx);
    deliver_or_retain_stall(&registry, &tx, "bot", note(3));
    assert_eq!(
        registry.lock().unwrap()["bot"]
            .pending_stall
            .as_ref()
            .unwrap()
            .sequence,
        3
    );
}

#[test]
fn take_stalled_snapshot_none_for_missing_no_workflow_complete_and_unknown_modes() {
    let registry = new_registry();
    assert!(take_stalled_snapshot(&registry, "missing").is_none());

    let mut entry = SubagentEntry::new("/tmp/bot.sock".into(), 1);
    entry.stalled_armed = true;
    registry.lock().unwrap().insert("bot".into(), entry);
    assert!(take_stalled_snapshot(&registry, "bot").is_none());

    for mode in ["complete", "unknown"] {
        let mut entry = SubagentEntry::new("/tmp/bot.sock".into(), 1);
        entry.stalled_armed = true;
        entry.workflow = Some(WorkflowSnapshot {
            mode: mode.into(),
            steps_completed: 1,
            steps_total: 1,
        });
        registry.lock().unwrap().insert(mode.into(), entry);
        assert!(take_stalled_snapshot(&registry, mode).is_none(), "{mode}");
    }
}
