// Swarm-coordinator parent wakes (#2467): once a child launched as a
// coordinator reports `swarm_state`, its turn-end note waits for the next
// one, which holds it or names the run's state.

use super::*;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentNotification, SwarmNoteState, new_notification_channel, new_registry,
};

/// A registry holding `id`, launched as a swarm coordinator.
fn registry_with(id: &str) -> crate::infrastructure::tools::subagent_registry::SubagentRegistry {
    let registry = new_registry();
    let mut entry = SubagentEntry::new(std::path::PathBuf::new(), 0);
    entry.coordinator_wake.launched = true;
    registry.lock().unwrap().insert(id.to_string(), entry);
    registry
}

fn agent_end() -> serde_json::Value {
    serde_json::json!({"type": "agent_end", "messages": []})
}

fn agent_start() -> serde_json::Value {
    serde_json::json!({"type": "agent_start"})
}

fn swarm_state(wake: &str, status: &str) -> serde_json::Value {
    serde_json::json!({"type": "swarm_state", "wake": wake, "status": status, "prompted": false})
}

fn prompted_state(wake: &str, status: &str) -> serde_json::Value {
    serde_json::json!({"type": "swarm_state", "wake": wake, "status": status, "prompted": true})
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
async fn a_child_not_launched_as_a_coordinator_keeps_its_turn_end_notes() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "worker".into(),
        SubagentEntry::new(std::path::PathBuf::new(), 0),
    );
    let (tx, mut rx) = new_notification_channel();
    apply_and_notify(&registry, Some(&tx), "worker", &agent_end());
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &swarm_state("hold", "running"),
    );
    apply_and_notify(&registry, Some(&tx), "worker", &agent_end());
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &swarm_state("hold", "running"),
    );
    let notes = drain(&mut rx);
    assert_eq!(notes.len(), 2, "{notes:?}");
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
async fn a_finished_run_wakes_the_parent_once_with_its_outcome() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("finished", "succeeded"),
    );
    let notes = drain(&mut rx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(
        notes[0],
        SubagentNotification::SwarmState {
            agent_id: "coord".into(),
            state: SwarmNoteState::Finished {
                status: "succeeded".into()
            },
        }
    );
    assert_eq!(
        notes[0].to_message(),
        "Swarm coordinator 'coord' reports its run succeeded; its final report is ready: agent_cmd get_messages."
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
        &swarm_state("idle", "paused"),
    );
    let notes = drain(&mut rx);
    assert_eq!(
        notes,
        vec![SubagentNotification::SwarmState {
            agent_id: "coord".into(),
            state: SwarmNoteState::Idle {
                status: "paused".into()
            },
        }]
    );
    assert_eq!(
        notes[0].to_message(),
        "Swarm coordinator 'coord' reports its run paused with nothing in flight and no result; it may need a decision: agent_cmd get_messages."
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
        &swarm_state("finished", "succeeded"),
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
    assert_eq!(
        notes[0].to_message(),
        "Swarm coordinator 'coord' has taken no turn for 30 min since its board last showed work in flight; check it with agent_cmd get_state or get_messages."
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

#[tokio::test]
async fn a_prompted_turn_s_reply_is_sent_while_workers_are_busy() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &prompted_state("hold", "running"),
    );
    let notes = drain(&mut rx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(matches!(notes[0], SubagentNotification::Completed { .. }));
}

#[tokio::test]
async fn an_unknown_wake_value_sends_the_ordinary_turn_end_note() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("stalled", "running"),
    );
    let notes = drain(&mut rx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(matches!(notes[0], SubagentNotification::Completed { .. }));
}

#[tokio::test(start_paused = true)]
async fn a_held_turn_end_no_state_settles_is_sent_as_it_was() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    tokio::task::yield_now().await;
    assert_eq!(
        drain(&mut rx),
        Vec::new(),
        "held until its state or the settle bound"
    );
    tokio::time::sleep(swarm_wake::SETTLE_AFTER + std::time::Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    let notes = drain(&mut rx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(matches!(notes[0], SubagentNotification::Completed { .. }));
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("idle", "running"),
    );
    assert_eq!(
        drain(&mut rx),
        Vec::new(),
        "a late state settles nothing twice"
    );
}

#[tokio::test(start_paused = true)]
async fn a_long_turn_in_the_same_drain_does_not_settle_the_held_note_early() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
    tokio::time::sleep(swarm_wake::SETTLE_AFTER * 3).await;
    tokio::task::yield_now().await;
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("hold", "running"),
    );
    tokio::task::yield_now().await;
    assert_eq!(drain(&mut rx), Vec::new());
}

#[tokio::test(start_paused = true)]
async fn a_state_other_than_hold_ends_the_quiet_report() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("finished", "succeeded"),
    );
    tokio::time::sleep(swarm_wake::QUIET_AFTER * 2).await;
    tokio::task::yield_now().await;
    assert_eq!(drain(&mut rx), Vec::new());
}

#[tokio::test(start_paused = true)]
async fn a_later_hold_replaces_the_earlier_quiet_report() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    tokio::time::sleep(swarm_wake::QUIET_AFTER / 2).await;
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("hold", "running"),
    );
    tokio::time::sleep(swarm_wake::QUIET_AFTER * 3 / 4).await;
    tokio::task::yield_now().await;
    assert_eq!(
        drain(&mut rx),
        Vec::new(),
        "the first hold's report is gone"
    );
    tokio::time::sleep(swarm_wake::QUIET_AFTER / 2).await;
    tokio::task::yield_now().await;
    let notes = drain(&mut rx);
    assert_eq!(notes.len(), 1, "{notes:?}");
}

#[tokio::test]
async fn a_run_error_while_a_turn_end_is_held_is_still_reported() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &serde_json::json!({"type":"response","command":"agent_error","error":"boom"}),
    );
    let notes = drain(&mut rx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        matches!(notes[0], SubagentNotification::Errored { .. }),
        "{notes:?}"
    );
}

/// Every sequenced note `rx` holds, in order.
fn drain_sequenced(
    rx: &mut crate::infrastructure::tools::subagent_registry::NotificationRx,
) -> Vec<crate::infrastructure::tools::subagent_registry::SequencedSubagentNotification> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

#[tokio::test(start_paused = true)]
async fn a_quiet_report_after_a_prompted_reply_carries_a_newer_sequence() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &prompted_state("hold", "running"),
    );
    let reply = drain_sequenced(&mut rx);
    assert_eq!(reply.len(), 1, "{reply:?}");
    tokio::time::sleep(swarm_wake::QUIET_AFTER + std::time::Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    let quiet = drain_sequenced(&mut rx);
    assert_eq!(quiet.len(), 1, "{quiet:?}");
    assert!(
        quiet[0].sequence > reply[0].sequence,
        "the parent's dedupe drops a note whose sequence is not newer"
    );
}

#[tokio::test(start_paused = true)]
async fn a_settled_turn_end_carries_a_newer_sequence_than_the_turn_end() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    let turn_end = registry.lock().unwrap()["coord"].notification_sequence;
    tokio::time::sleep(swarm_wake::SETTLE_AFTER + std::time::Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    let notes = drain_sequenced(&mut rx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].sequence > turn_end);
}

#[tokio::test(start_paused = true)]
async fn a_coordinator_that_exited_reports_no_quiet() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    registry.lock().unwrap().get_mut("coord").unwrap().status =
        crate::infrastructure::tools::subagent_registry::SubagentStatus::Exited;
    tokio::time::sleep(swarm_wake::QUIET_AFTER * 2).await;
    tokio::task::yield_now().await;
    assert_eq!(drain(&mut rx), Vec::new());
}

#[tokio::test]
async fn a_paused_run_wakes_the_parent_as_needing_a_decision() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("paused", "paused"),
    );
    let notes = drain(&mut rx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(
        notes[0].to_message(),
        "Swarm coordinator 'coord' reports its run paused with no result; it may need a decision: agent_cmd get_messages."
    );
}

#[tokio::test]
async fn a_finished_state_naming_no_status_sends_the_ordinary_note() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &serde_json::json!({"type": "swarm_state", "wake": "finished"}),
    );
    let notes = drain(&mut rx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(matches!(notes[0], SubagentNotification::Completed { .. }));
}

// `swarm_state` must survive the monitor's substring pre-filter: this
// enters through `handle_monitor_line`, the real wire path.
#[tokio::test]
async fn a_swarm_state_line_from_the_wire_settles_the_held_turn_end() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    handle_monitor_line(
        r#"{"type":"swarm_state","wake":"idle","status":"running","prompted":false}"#,
        "coord",
        &registry,
        Some(&tx),
        None,
        None,
    );
    let notes = drain(&mut rx);
    assert_eq!(
        notes,
        vec![SubagentNotification::SwarmState {
            agent_id: "coord".into(),
            state: SwarmNoteState::Idle {
                status: "running".into()
            },
        }]
    );
}

fn agent_error() -> serde_json::Value {
    serde_json::json!({"type":"response","command":"agent_error","error":"rate limited"})
}

#[tokio::test(start_paused = true)]
async fn a_failed_turn_is_reported_once_by_the_next_state_and_never_settled_over() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
    apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
    apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
    apply_and_notify(&registry, Some(&tx), "coord", &agent_error());
    let errored = drain(&mut rx);
    assert!(
        matches!(errored[..], [SubagentNotification::Errored { .. }]),
        "{errored:?}"
    );
    tokio::time::sleep(swarm_wake::SETTLE_AFTER * 2).await;
    tokio::task::yield_now().await;
    assert_eq!(
        drain(&mut rx),
        Vec::new(),
        "no turn end is settled over the error"
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("hold", "running"),
    );
    assert_eq!(
        drain(&mut rx),
        vec![SubagentNotification::SwarmState {
            agent_id: "coord".into(),
            state: SwarmNoteState::Stopped,
        }]
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("hold", "running"),
    );
    assert_eq!(drain(&mut rx), Vec::new(), "reported once");
}

#[tokio::test]
async fn a_failed_turn_before_an_idle_run_names_the_idle_run() {
    let registry = registry_with("coord");
    let (tx, mut rx) = new_notification_channel();
    first_boundary(&registry, &tx, &mut rx);
    apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
    apply_and_notify(&registry, Some(&tx), "coord", &agent_error());
    drain(&mut rx);
    apply_and_notify(
        &registry,
        Some(&tx),
        "coord",
        &swarm_state("idle", "running"),
    );
    assert_eq!(
        drain(&mut rx),
        vec![SubagentNotification::SwarmState {
            agent_id: "coord".into(),
            state: SwarmNoteState::Idle {
                status: "running".into()
            },
        }]
    );
}

#[tokio::test]
async fn a_turn_that_succeeds_after_a_failed_one_is_not_reported_as_stopped() {
    for (state, expected_notes) in [
        (swarm_state("hold", "running"), 0),
        (prompted_state("hold", "running"), 1),
    ] {
        let registry = registry_with("coord");
        let (tx, mut rx) = new_notification_channel();
        first_boundary(&registry, &tx, &mut rx);
        apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
        apply_and_notify(&registry, Some(&tx), "coord", &agent_error());
        drain(&mut rx);
        apply_and_notify(&registry, Some(&tx), "coord", &agent_start());
        apply_and_notify(&registry, Some(&tx), "coord", &agent_end());
        apply_and_notify(&registry, Some(&tx), "coord", &state);
        let notes = drain(&mut rx);
        assert_eq!(notes.len(), expected_notes, "{notes:?}");
        assert!(
            notes
                .iter()
                .all(|n| matches!(n, SubagentNotification::Completed { .. })),
            "{notes:?}"
        );
    }
}
