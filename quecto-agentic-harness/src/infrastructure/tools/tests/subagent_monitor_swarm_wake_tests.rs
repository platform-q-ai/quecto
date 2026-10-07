// Swarm-coordinator parent wakes (#2467): once a child reports `swarm_state`,
// its turn-end note waits for the next one, which holds it or names the
// run's state.

use super::*;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentNotification, SwarmNoteState, new_notification_channel, new_registry,
};

fn registry_with(id: &str) -> crate::infrastructure::tools::subagent_registry::SubagentRegistry {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        id.to_string(),
        SubagentEntry::new(std::path::PathBuf::new(), 0),
    );
    registry
}

fn agent_end() -> serde_json::Value {
    serde_json::json!({"type": "agent_end", "messages": []})
}

fn agent_start() -> serde_json::Value {
    serde_json::json!({"type": "agent_start"})
}

fn swarm_state(wake: &str, status: &str) -> serde_json::Value {
    serde_json::json!({"type": "swarm_state", "wake": wake, "status": status})
}

fn drain(
    rx: &mut crate::infrastructure::tools::subagent_registry::NotificationRx,
) -> Vec<SubagentNotification> {
    let mut notes = Vec::new();
    while let Ok(n) = rx.try_recv() {
        notes.push(n.notification);
    }
    notes
}

/// Reports one idle boundary to make `coord` a reporting coordinator, and
/// drains what that first boundary sent (its ordinary turn-end note).
fn first_boundary(
    registry: &crate::infrastructure::tools::subagent_registry::SubagentRegistry,
    tx: &crate::infrastructure::tools::subagent_registry::NotificationTx,
    rx: &mut crate::infrastructure::tools::subagent_registry::NotificationRx,
) {
    apply_and_notify(registry, Some(tx), "coord", &agent_end());
    apply_and_notify(registry, Some(tx), "coord", &swarm_state("hold", "running"));
    let notes = drain(rx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(matches!(notes[0], SubagentNotification::Completed { .. }));
}

#[tokio::test]
async fn a_child_that_never_reports_swarm_state_keeps_its_turn_end_note() {
    let registry = registry_with("worker");
    let (tx, mut rx) = new_notification_channel();
    apply_and_notify(&registry, Some(&tx), "worker", &agent_end());
    apply_and_notify(&registry, Some(&tx), "worker", &agent_end());
    let notes = drain(&mut rx);
    assert_eq!(notes.len(), 2);
    assert!(
        notes
            .iter()
            .all(|n| matches!(n, SubagentNotification::Completed { .. }))
    );
}

#[tokio::test]
async fn a_held_idle_boundary_wakes_nobody() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    for _ in 0..3 {
        apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
        apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    }
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("hold", "running"),
    );
    assert_eq!(drain(&mut rx), Vec::new());
}

#[tokio::test]
async fn a_finished_run_wakes_the_parent_once_with_its_status() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("finished", "complete"),
    );
    let notes = drain(&mut rx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(
        notes[0],
        SubagentNotification::SwarmState {
            agent_id: "coord".into(),
            state: SwarmNoteState::Finished {
                status: "complete".into()
            },
        }
    );
    assert!(
        notes[0].to_message().contains("complete"),
        "{}",
        notes[0].to_message()
    );
}

#[tokio::test]
async fn an_idle_run_wakes_the_parent_as_needing_a_decision() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("idle", "running"),
    );
    let notes = drain(&mut rx);
    assert_eq!(
        notes,
        vec![SubagentNotification::SwarmState {
            agent_id: "coord".into(),
            state: SwarmNoteState::Idle,
        }]
    );
}

#[tokio::test]
async fn an_unreadable_board_sends_the_ordinary_turn_end_note() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(&registry, Some(&tx), "coord", &swarm_state("unknown", ""));
    let notes = drain(&mut rx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(matches!(notes[0], SubagentNotification::Completed { .. }));
}

#[tokio::test]
async fn a_boundary_with_no_held_turn_end_wakes_nobody() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("finished", "complete"),
    );
    assert_eq!(drain(&mut rx), Vec::new());
}

#[tokio::test(start_paused = true)]
async fn a_hold_with_no_new_turn_reports_a_quiet_coordinator_once() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("hold", "running"),
    );
    tokio::time::sleep(swarm_wake::QUIET_AFTER + std::time::Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    let notes = drain(&mut rx);
    assert_eq!(
        notes,
        vec![SubagentNotification::SwarmState {
            agent_id: "coord".into(),
            state: SwarmNoteState::Quiet { minutes: 30 },
        }]
    );
    tokio::time::sleep(swarm_wake::QUIET_AFTER * 2).await;
    tokio::task::yield_now().await;
    assert_eq!(
        drain(&mut rx),
        Vec::new(),
        "quiet is reported once per hold"
    );
}

#[tokio::test(start_paused = true)]
async fn a_new_turn_cancels_the_quiet_report() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("hold", "running"),
    );
    tokio::time::sleep(swarm_wake::QUIET_AFTER / 2).await;
    apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
    tokio::time::sleep(swarm_wake::QUIET_AFTER).await;
    tokio::task::yield_now().await;
    assert_eq!(drain(&mut rx), Vec::new());
}
