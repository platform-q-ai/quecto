// #1082 review round 2: staleness races on stalls retained under a saturated
// notification channel. A retained stall must be invalidated by any lifecycle
// event that supersedes it (agent_error, agent_start, workflow progress), and
// the incoming event must be applied BEFORE retained stalls are retried.

use super::*;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentNotification, new_registry,
};

fn insert_entry(
    registry: &crate::infrastructure::tools::subagent_registry::SubagentRegistry,
    id: &str,
) {
    let mut guard = registry.lock().unwrap();
    guard.insert(
        id.to_string(),
        SubagentEntry::new(std::path::PathBuf::new(), 0),
    );
}

// Retain a stall for "worker" under a saturated capacity-1 channel and return
// (tx, rx) with the occupant still in the channel.
fn retain_saturated_stall(
    registry: &crate::infrastructure::tools::subagent_registry::SubagentRegistry,
) -> (
    crate::infrastructure::tools::subagent_registry::NotificationTx,
    tokio::sync::mpsc::Receiver<
        crate::infrastructure::tools::subagent_registry::SequencedSubagentNotification,
    >,
) {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    tx.try_send(
        crate::infrastructure::tools::subagent_registry::SequencedSubagentNotification::new(
            99,
            SubagentNotification::Completed {
                agent_id: "other".to_string(),
            },
        ),
    )
    .expect("fill bounded channel");
    apply_and_notify(
        registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_state","mode":"active","progress":{"done":3,"total":7}}),
    );
    apply_and_notify(
        registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_idle","reason":"exhausted"}),
    );
    assert!(
        registry
            .lock()
            .unwrap()
            .get("worker")
            .is_some_and(|entry| entry.pending_stall.is_some()),
        "precondition: saturated stall must be retained"
    );
    (tx, rx)
}

// Drain every remaining notification and assert none is a Stalled verdict.
async fn assert_no_stall_delivered(
    rx: &mut tokio::sync::mpsc::Receiver<
        crate::infrastructure::tools::subagent_registry::SequencedSubagentNotification,
    >,
) {
    // Let the capacity backstop task (if any) observe the freed capacity.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    while let Ok(n) = rx.try_recv() {
        assert!(
            !matches!(n.notification, SubagentNotification::Stalled { .. }),
            "an invalidated retained stall must never be delivered"
        );
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }
}

// (High): a stall retained under saturation is invalidated by a run-level
// error arriving BEFORE capacity frees — neither the event-driven retry nor
// the capacity backstop may deliver it.
#[tokio::test]
async fn retained_stall_is_invalidated_by_agent_error_before_capacity_frees() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    insert_entry(&registry, "other");
    let (tx, mut rx) = retain_saturated_stall(&registry);
    // The error arrives while the channel is still full.
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"response","command":"agent_error","error":"boom"}),
    );
    assert!(
        registry
            .lock()
            .unwrap()
            .get("worker")
            .is_some_and(|entry| entry.pending_stall.is_none()),
        "agent_error must invalidate the retained stall"
    );
    rx.try_recv().expect("free channel capacity");
    // Another agent's event drives the retry path; nothing must fire.
    apply_and_notify(
        &registry,
        Some(&tx),
        "other",
        &serde_json::json!({"type":"tool_execution_start","toolName":"read"}),
    );
    assert_no_stall_delivered(&mut rx).await;
}

// (High): a new run (agent_start) supersedes a stall retained from the
// previous run.
#[tokio::test]
async fn retained_stall_is_invalidated_by_agent_start_before_capacity_frees() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    insert_entry(&registry, "other");
    let (tx, mut rx) = retain_saturated_stall(&registry);
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"agent_start"}),
    );
    rx.try_recv().expect("free channel capacity");
    apply_and_notify(
        &registry,
        Some(&tx),
        "other",
        &serde_json::json!({"type":"tool_execution_start","toolName":"read"}),
    );
    assert_no_stall_delivered(&mut rx).await;
}

// (High): fresh workflow progress supersedes a retained stall — the retained
// snapshot no longer describes current state.
#[tokio::test]
async fn retained_stall_is_invalidated_by_workflow_progress_before_capacity_frees() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    insert_entry(&registry, "other");
    let (tx, mut rx) = retain_saturated_stall(&registry);
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_state","mode":"active","progress":{"done":4,"total":7}}),
    );
    rx.try_recv().expect("free channel capacity");
    apply_and_notify(
        &registry,
        Some(&tx),
        "other",
        &serde_json::json!({"type":"tool_execution_start","toolName":"read"}),
    );
    assert_no_stall_delivered(&mut rx).await;
}

// (High): the incoming lifecycle event must be applied BEFORE retained stalls
// are retried — an agent_error arriving at the exact moment capacity is
// already free must not first publish the obsolete stall.
#[tokio::test]
async fn same_event_that_invalidates_does_not_first_deliver_the_stall() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = retain_saturated_stall(&registry);
    // Free capacity FIRST, then deliver the error: the retry runs inside this
    // same apply_and_notify call and must observe the already-applied error.
    rx.try_recv().expect("free channel capacity");
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"response","command":"agent_error","error":"boom"}),
    );
    assert_no_stall_delivered(&mut rx).await;
}

// #1082 review round 3 (High): child exit supersedes a retained stall — the
// child is gone, so neither the event-driven retry nor the capacity backstop
// may deliver the obsolete alert after the exit.
#[tokio::test]
async fn retained_stall_is_invalidated_by_child_exit_before_capacity_frees() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    insert_entry(&registry, "other");
    let (tx, mut rx) = retain_saturated_stall(&registry);
    // The exit happens while the channel is still full.
    {
        let mut guard = registry.lock().unwrap();
        let entry = guard.get_mut("worker").expect("worker entry");
        mark_exited(entry);
        assert!(
            entry.pending_stall.is_none(),
            "exit must invalidate the retained stall"
        );
    }
    rx.try_recv().expect("free channel capacity");
    // Another agent's event drives the retry path; nothing must fire.
    apply_and_notify(
        &registry,
        Some(&tx),
        "other",
        &serde_json::json!({"type":"tool_execution_start","toolName":"read"}),
    );
    assert_no_stall_delivered(&mut rx).await;
}

#[tokio::test]
async fn exhausted_idle_while_direct_descendant_active_preserves_stall_latch() {
    let registry = new_registry();
    insert_entry(&registry, "parent");
    insert_entry(&registry, "child");
    {
        let mut guard = registry.lock().unwrap();
        guard.get_mut("parent").unwrap().workflow = Some(
            crate::infrastructure::tools::subagent_registry::WorkflowSnapshot {
                mode: "active".into(),
                steps_completed: 2,
                steps_total: 5,
            },
        );
        let child = guard.get_mut("child").unwrap();
        child.parent_id = Some("parent".into());
        child.status = SubagentStatus::Running;
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    apply_and_notify(
        &registry,
        Some(&tx),
        "parent",
        &serde_json::json!({"type":"workflow_idle","reason":"exhausted"}),
    );
    assert!(
        rx.try_recv().is_err(),
        "active descendant means no stall note"
    );
    assert!(
        registry
            .lock()
            .unwrap()
            .get("parent")
            .unwrap()
            .stalled_armed,
        "latch must remain armed while runtime is effectively active"
    );
}

#[tokio::test]
async fn exhausted_idle_while_transitive_descendant_active_preserves_stall_latch_then_later_stalls()
{
    let registry = new_registry();
    for id in ["parent", "child", "grandchild"] {
        insert_entry(&registry, id);
    }
    {
        let mut guard = registry.lock().unwrap();
        guard.get_mut("parent").unwrap().workflow = Some(
            crate::infrastructure::tools::subagent_registry::WorkflowSnapshot {
                mode: "active".into(),
                steps_completed: 2,
                steps_total: 5,
            },
        );
        guard.get_mut("child").unwrap().parent_id = Some("parent".into());
        let grandchild = guard.get_mut("grandchild").unwrap();
        grandchild.parent_id = Some("child".into());
        grandchild.status = SubagentStatus::Running;
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    apply_and_notify(
        &registry,
        Some(&tx),
        "parent",
        &serde_json::json!({"type":"workflow_idle","reason":"exhausted"}),
    );
    assert!(rx.try_recv().is_err());
    assert!(
        registry
            .lock()
            .unwrap()
            .get("parent")
            .unwrap()
            .stalled_armed
    );
    {
        let mut guard = registry.lock().unwrap();
        guard.get_mut("child").unwrap().status = SubagentStatus::Idle;
        guard.get_mut("grandchild").unwrap().status = SubagentStatus::Idle;
    }
    apply_and_notify(
        &registry,
        Some(&tx),
        "parent",
        &serde_json::json!({"type":"workflow_idle","reason":"exhausted"}),
    );
    assert!(matches!(
        rx.try_recv().unwrap().notification,
        SubagentNotification::Stalled { .. }
    ));
}

#[tokio::test]
async fn terminal_completion_and_resumed_progress_do_not_create_stale_stall_after_active_descendant()
 {
    let registry = new_registry();
    insert_entry(&registry, "parent");
    insert_entry(&registry, "child");
    {
        let mut guard = registry.lock().unwrap();
        guard.get_mut("parent").unwrap().workflow = Some(
            crate::infrastructure::tools::subagent_registry::WorkflowSnapshot {
                mode: "active".into(),
                steps_completed: 2,
                steps_total: 5,
            },
        );
        let child = guard.get_mut("child").unwrap();
        child.parent_id = Some("parent".into());
        child.status = SubagentStatus::Running;
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    apply_and_notify(
        &registry,
        Some(&tx),
        "parent",
        &serde_json::json!({"type":"workflow_idle","reason":"exhausted"}),
    );
    registry.lock().unwrap().get_mut("child").unwrap().status = SubagentStatus::Idle;
    apply_and_notify(
        &registry,
        Some(&tx),
        "parent",
        &serde_json::json!({"type":"workflow_state","mode":"complete","progress":{"done":5,"total":5}}),
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "parent",
        &serde_json::json!({"type":"workflow_idle","reason":"completed"}),
    );
    assert!(
        !std::iter::from_fn(|| rx.try_recv().ok())
            .any(|n| matches!(n.notification, SubagentNotification::Stalled { .. }))
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "parent",
        &serde_json::json!({"type":"workflow_state","mode":"active","progress":{"done":3,"total":5}}),
    );
    assert!(
        registry
            .lock()
            .unwrap()
            .get("parent")
            .unwrap()
            .stalled_armed
    );
}

#[tokio::test]
async fn unrelated_active_agent_does_not_suppress_genuine_stall() {
    let registry = new_registry();
    insert_entry(&registry, "parent");
    insert_entry(&registry, "unrelated");
    {
        let mut guard = registry.lock().unwrap();
        guard.get_mut("parent").unwrap().workflow = Some(
            crate::infrastructure::tools::subagent_registry::WorkflowSnapshot {
                mode: "active".into(),
                steps_completed: 1,
                steps_total: 2,
            },
        );
        guard.get_mut("unrelated").unwrap().status = SubagentStatus::Running;
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    apply_and_notify(
        &registry,
        Some(&tx),
        "parent",
        &serde_json::json!({"type":"workflow_idle","reason":"exhausted"}),
    );
    assert!(matches!(
        rx.try_recv().unwrap().notification,
        SubagentNotification::Stalled { .. }
    ));
}

#[tokio::test]
async fn descendant_identity_session_key_forms_suppress_parent_stall() {
    let registry = new_registry();
    insert_entry(&registry, "uuid-parent");
    insert_entry(&registry, "child");
    {
        let mut guard = registry.lock().unwrap();
        let parent = guard.get_mut("uuid-parent").unwrap();
        parent.display_name = "display-parent".into();
        parent.workflow = Some(
            crate::infrastructure::tools::subagent_registry::WorkflowSnapshot {
                mode: "active".into(),
                steps_completed: 1,
                steps_total: 2,
            },
        );
        let parent_uuid = parent.agent_uuid.to_string();
        let child = guard.get_mut("child").unwrap();
        child.parent_id = Some(parent_uuid);
        child.status = SubagentStatus::Running;
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    apply_and_notify(
        &registry,
        Some(&tx),
        "uuid-parent",
        &serde_json::json!({"type":"workflow_idle","reason":"exhausted"}),
    );
    assert!(rx.try_recv().is_err());
    assert!(
        registry
            .lock()
            .unwrap()
            .get("uuid-parent")
            .unwrap()
            .stalled_armed
    );
}
