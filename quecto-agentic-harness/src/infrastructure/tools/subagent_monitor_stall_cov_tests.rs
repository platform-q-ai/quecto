use super::*;
use crate::infrastructure::tools::subagent_registry::{
    SequencedSubagentNotification, SubagentEntry, SubagentNotification, WorkflowSnapshot,
    new_registry,
};

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
