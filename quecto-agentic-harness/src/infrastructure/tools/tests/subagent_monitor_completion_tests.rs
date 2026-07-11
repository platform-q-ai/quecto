// Terminal-completion notification tests (#904): a `Completed` note must fire
// only on TRUE terminal completion — workflow `complete`, or a non-workflow
// agent's turn-end — NOT on every per-step/per-turn `agent_end`.

use super::*;
use crate::domain::workflow::WorkflowMode;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentNotification, new_notification_channel, new_registry,
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

#[test]
fn agent_end_terminal_when_no_workflow() {
    // No workflow → a turn-end is a logical completion.
    assert!(agent_end_is_terminal(None));
}

#[test]
fn agent_end_not_terminal_mid_workflow() {
    // Active (mid-workflow) step-end must NOT notify.
    assert!(!agent_end_is_terminal(Some(
        WorkflowMode::Active.wire_str()
    )));
    assert!(!agent_end_is_terminal(Some(
        WorkflowMode::SelectingTemplate.wire_str()
    )));
}

#[test]
fn agent_end_terminal_when_workflow_complete() {
    assert!(agent_end_is_terminal(Some(
        WorkflowMode::Complete.wire_str()
    )));
}

// AC5: a monitored agent emitting multiple `agent_end` events across a workflow
// (each step ends with its own agent_end) yields exactly ONE `Completed`
// notification — at terminal (workflow `complete`) completion.
#[tokio::test]
async fn workflow_emits_single_completion_across_steps() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();

    let wf = |mode: &str, done: u64, total: u64| {
        serde_json::json!({
            "type": "workflow_state",
            "mode": mode,
            "progress": {"done": done, "total": total}
        })
    };
    let agent_end = serde_json::json!({
        "type": "agent_end",
        "messages": [{"role": "assistant", "content": "step done"}]
    });

    // initial turn + 2 active steps, each ending with agent_end while active.
    for _ in 0..3 {
        apply_and_notify(&registry, Some(&tx), "worker", &wf("active", 1, 2));
        apply_and_notify(&registry, Some(&tx), "worker", &agent_end);
    }
    // terminal: workflow completes, then the final agent_end.
    apply_and_notify(&registry, Some(&tx), "worker", &wf("complete", 2, 2));
    apply_and_notify(&registry, Some(&tx), "worker", &agent_end);

    let mut completions = 0;
    while let Ok(n) = rx.try_recv() {
        if matches!(n.notification, SubagentNotification::Completed { .. }) {
            completions += 1;
        }
    }
    assert_eq!(
        completions, 1,
        "expected exactly one Completed note across an N-step workflow"
    );
}

// AC1 / finding #1: with the default `completion_nudge=true`, the kernel runs ONE
// extra "report your result and stop" turn AFTER the workflow reaches `complete`.
// That follow-up turn emits its own agent_start + (still-`complete`) workflow_state
// + agent_end. The terminal-completion latch must suppress the nudge turn's
// agent_end so the run still yields exactly ONE Completed note.
#[tokio::test]
async fn completion_nudge_followup_turn_does_not_double_notify() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();

    let wf = |mode: &str, done: u64, total: u64| {
        serde_json::json!({
            "type": "workflow_state",
            "mode": mode,
            "progress": {"done": done, "total": total}
        })
    };
    let agent_start = serde_json::json!({"type": "agent_start"});
    let agent_end = serde_json::json!({
        "type": "agent_end",
        "messages": [{"role": "assistant", "content": "done"}]
    });

    // Completing turn: step finishes, workflow → complete, terminal agent_end.
    apply_and_notify(&registry, Some(&tx), "worker", &agent_start);
    apply_and_notify(&registry, Some(&tx), "worker", &wf("complete", 2, 2));
    apply_and_notify(&registry, Some(&tx), "worker", &agent_end);

    // completion_nudge follow-up turn: fresh agent_start, workflow STILL complete,
    // another agent_end. This must NOT fire a second Completed note.
    apply_and_notify(&registry, Some(&tx), "worker", &agent_start);
    apply_and_notify(&registry, Some(&tx), "worker", &wf("complete", 2, 2));
    apply_and_notify(&registry, Some(&tx), "worker", &agent_end);

    let mut completions = 0;
    while let Ok(n) = rx.try_recv() {
        if matches!(n.notification, SubagentNotification::Completed { .. }) {
            completions += 1;
        }
    }
    assert_eq!(
        completions, 1,
        "completion_nudge follow-up turn must not produce a second Completed note"
    );
}

// A workflow that genuinely re-runs (leaves `complete` back to `active`, then
// re-completes) must re-arm the latch and notify again on the new completion.
#[tokio::test]
async fn rerun_after_complete_notifies_again() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();

    let wf = |mode: &str| {
        serde_json::json!({
            "type": "workflow_state",
            "mode": mode,
            "progress": {"done": 2, "total": 2}
        })
    };
    let agent_end = serde_json::json!({
        "type": "agent_end",
        "messages": [{"role": "assistant", "content": "done"}]
    });

    apply_and_notify(&registry, Some(&tx), "worker", &wf("complete"));
    apply_and_notify(&registry, Some(&tx), "worker", &agent_end);
    // Re-run: workflow goes back to active (re-arms), then completes again.
    apply_and_notify(&registry, Some(&tx), "worker", &wf("active"));
    apply_and_notify(&registry, Some(&tx), "worker", &wf("complete"));
    apply_and_notify(&registry, Some(&tx), "worker", &agent_end);

    let mut completions = 0;
    while let Ok(n) = rx.try_recv() {
        if matches!(n.notification, SubagentNotification::Completed { .. }) {
            completions += 1;
        }
    }
    assert_eq!(
        completions, 2,
        "a genuine re-run after complete must re-arm and notify again"
    );
}

// #1076: an unfinished workflow that reaches a stable idle boundary requires
// supervision instead of being silently treated as routine step progress.
#[tokio::test]
async fn active_workflow_without_continuation_emits_actionable_stall() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_state","mode":"active","progress":{"done":3,"total":7}}),
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_idle","reason":"exhausted"}),
    );

    let note = rx
        .try_recv()
        .expect("stable unfinished workflow must alert");
    let message = note.notification.to_message();
    assert!(message.contains("stalled"));
    assert!(message.contains("3/7"));
    assert!(message.contains("prompt, steer, abort, or kill"));
    assert!(rx.try_recv().is_err(), "stall must be emitted exactly once");
}

#[tokio::test]
async fn routine_agent_end_without_stable_idle_signal_stays_silent() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_state","mode":"active","progress":{"done":3,"total":7}}),
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"agent_end","messages":[]}),
    );

    assert!(
        rx.try_recv().is_err(),
        "ambiguous legacy/routine agent_end must not produce a false stall"
    );
}

#[tokio::test]
async fn repeated_stable_idle_in_same_stalled_state_alerts_once() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();
    let workflow = serde_json::json!({
        "type":"workflow_state",
        "mode":"active",
        "progress":{"done":3,"total":7}
    });
    let agent_end = serde_json::json!({"type":"workflow_idle","reason":"exhausted"});

    apply_and_notify(&registry, Some(&tx), "worker", &workflow);
    apply_and_notify(&registry, Some(&tx), "worker", &agent_end);
    apply_and_notify(&registry, Some(&tx), "worker", &agent_end);

    assert!(matches!(
        rx.try_recv().expect("first stall must alert").notification,
        SubagentNotification::Stalled { .. }
    ));
    assert!(rx.try_recv().is_err(), "unchanged stall must not re-alert");
}

#[tokio::test]
async fn workflow_progress_rearms_stalled_alert() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();
    let agent_end = serde_json::json!({"type":"workflow_idle","reason":"exhausted"});

    for done in [3, 4] {
        apply_and_notify(
            &registry,
            Some(&tx),
            "worker",
            &serde_json::json!({
                "type":"workflow_state",
                "mode":"active",
                "progress":{"done":done,"total":7}
            }),
        );
        apply_and_notify(&registry, Some(&tx), "worker", &agent_end);
    }

    let alerts = std::iter::from_fn(|| rx.try_recv().ok())
        .filter(|note| matches!(note.notification, SubagentNotification::Stalled { .. }))
        .count();
    assert_eq!(
        alerts, 2,
        "new workflow progress must re-arm the stall latch"
    );
}

#[tokio::test]
async fn agent_start_rearms_stalled_alert() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();
    let workflow = serde_json::json!({
        "type":"workflow_state",
        "mode":"active",
        "progress":{"done":3,"total":7}
    });
    let agent_end = serde_json::json!({"type":"workflow_idle","reason":"exhausted"});

    apply_and_notify(&registry, Some(&tx), "worker", &workflow);
    apply_and_notify(&registry, Some(&tx), "worker", &agent_end);
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"agent_start"}),
    );
    apply_and_notify(&registry, Some(&tx), "worker", &agent_end);

    let alerts = std::iter::from_fn(|| rx.try_recv().ok())
        .filter(|note| matches!(note.notification, SubagentNotification::Stalled { .. }))
        .count();
    assert_eq!(alerts, 2, "a new run must re-arm the stall latch");
}

#[tokio::test]
async fn saturated_channel_retries_exact_stall_on_next_monitor_event() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tx.try_send(
        crate::infrastructure::tools::subagent_registry::SequencedSubagentNotification::new(
            99,
            SubagentNotification::Completed {
                agent_id: "other".to_string(),
                summary: "occupy channel".to_string(),
            },
        ),
    )
    .expect("fill bounded channel");

    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({
            "type":"workflow_state",
            "mode":"active",
            "progress":{"done":3,"total":7}
        }),
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_idle","reason":"exhausted"}),
    );
    let pending = registry
        .lock()
        .unwrap()
        .get("worker")
        .and_then(|entry| entry.pending_stall.clone())
        .expect("saturated stall must remain retryable");
    assert_eq!(pending.sequence, 2);

    rx.try_recv().expect("free channel capacity");
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"tool_execution_start","toolName":"read"}),
    );

    let delivered = rx.try_recv().expect("pending stall must be retried");
    assert_eq!(delivered, pending, "retry must preserve exact sequence");
    assert!(matches!(
        delivered.notification,
        SubagentNotification::Stalled { .. }
    ));
    assert!(
        registry
            .lock()
            .unwrap()
            .get("worker")
            .is_some_and(|entry| entry.pending_stall.is_none()),
        "accepted retry must clear pending state"
    );
    assert!(rx.try_recv().is_err(), "stall retry must be exactly once");
}

#[tokio::test]
async fn selecting_template_without_continuation_emits_stall() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_state","mode":"selecting_template","progress":{"done":0,"total":0}}),
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_idle","reason":"exhausted"}),
    );
    let notification = rx
        .try_recv()
        .expect("template selection stall must alert")
        .notification;
    assert!(matches!(
        notification,
        SubagentNotification::Stalled {
            ref workflow_mode,
            steps_completed: 0,
            steps_total: 0,
            ..
        } if workflow_mode == "selecting_template"
    ));
}

#[tokio::test]
async fn seeded_bound_entry_does_not_complete_before_first_workflow_state() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    registry.lock().unwrap().get_mut("worker").unwrap().workflow = Some(
        crate::infrastructure::tools::subagent_registry::WorkflowSnapshot {
            mode: "active".to_string(),
            steps_completed: 0,
            steps_total: 2,
        },
    );
    let (tx, mut rx) = new_notification_channel();
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"agent_end","messages":[]}),
    );
    assert!(
        rx.try_recv().is_err(),
        "spawn-seeded binding must prevent premature plain completion"
    );
}

// AC2: a non-workflow agent emits a completion on its turn-end.
#[tokio::test]
async fn non_workflow_agent_emits_completion_on_turn_end() {
    let registry = new_registry();
    insert_entry(&registry, "solo");
    let (tx, mut rx) = new_notification_channel();
    let agent_end = serde_json::json!({
        "type": "agent_end",
        "messages": [{"role": "assistant", "content": "all done"}]
    });
    apply_and_notify(&registry, Some(&tx), "solo", &agent_end);

    let n = rx.try_recv().expect("expected a notification");
    assert!(matches!(
        n.notification,
        SubagentNotification::Completed { .. }
    ));
}

// AC3: errored notifications are unaffected by the terminal-completion gating.
#[tokio::test]
async fn tool_error_still_notifies_mid_workflow() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type": "workflow_state", "mode": "active", "progress": {"done": 0, "total": 2}}),
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type": "tool_execution_end", "isError": true, "toolName": "bash"}),
    );
    let n = rx.try_recv().expect("expected an error notification");
    assert!(matches!(
        n.notification,
        SubagentNotification::Errored { .. }
    ));
}

// Review fix (#1082): `workflow_idle` must survive the monitor's cheap
// substring pre-filter. This enters through `handle_monitor_line` — the real
// wire path — so it fails if the event is missing from STATE_CHANGING_EVENTS,
// unlike the `apply_and_notify`-level tests above.
#[tokio::test]
async fn workflow_idle_line_from_wire_emits_stall() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();

    handle_monitor_line(
        r#"{"type":"workflow_state","mode":"active","progress":{"done":1,"total":3}}"#,
        "worker",
        &registry,
        Some(&tx),
        None,
        None,
    );
    handle_monitor_line(
        r#"{"type":"workflow_idle","reason":"exhausted"}"#,
        "worker",
        &registry,
        Some(&tx),
        None,
        None,
    );

    let stalled = std::iter::from_fn(|| rx.try_recv().ok())
        .any(|n| matches!(n.notification, SubagentNotification::Stalled { .. }));
    assert!(
        stalled,
        "a workflow_idle wire line must pass the pre-filter and raise a stall alert"
    );
}

// Review fix (#1082): a retained stall must not depend on the stalled child's
// own (nonexistent) future events. Any OTHER agent's monitor event retries it.
#[tokio::test]
async fn saturated_stall_retried_by_another_agents_event() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    insert_entry(&registry, "other");
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tx.try_send(
        crate::infrastructure::tools::subagent_registry::SequencedSubagentNotification::new(
            99,
            SubagentNotification::Completed {
                agent_id: "other".to_string(),
                summary: "occupy channel".to_string(),
            },
        ),
    )
    .expect("fill bounded channel");

    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_state","mode":"active","progress":{"done":3,"total":7}}),
    );
    apply_and_notify(
        &registry,
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
        "saturated stall must be retained"
    );

    rx.try_recv().expect("free channel capacity");
    // The stalled worker stays silent; a DIFFERENT agent's event must retry.
    apply_and_notify(
        &registry,
        Some(&tx),
        "other",
        &serde_json::json!({"type":"tool_execution_start","toolName":"read"}),
    );

    let delivered = rx
        .try_recv()
        .expect("cross-agent retry must deliver the stall");
    assert!(matches!(
        delivered.notification,
        SubagentNotification::Stalled { .. }
    ));
    assert!(
        registry
            .lock()
            .unwrap()
            .get("worker")
            .is_some_and(|entry| entry.pending_stall.is_none()),
        "delivered retry must clear pending state"
    );
    assert!(
        rx.try_recv().is_err(),
        "stall must be delivered exactly once"
    );
}

// Review fix (#1082): with NO further monitor events anywhere in the fleet, the
// capacity backstop alone must deliver the retained stall once the parent
// drains the channel.
#[tokio::test]
async fn saturated_stall_delivered_by_backstop_without_any_further_events() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tx.try_send(
        crate::infrastructure::tools::subagent_registry::SequencedSubagentNotification::new(
            99,
            SubagentNotification::Completed {
                agent_id: "other".to_string(),
                summary: "occupy channel".to_string(),
            },
        ),
    )
    .expect("fill bounded channel");

    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_state","mode":"active","progress":{"done":3,"total":7}}),
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_idle","reason":"exhausted"}),
    );

    // Drain the occupying note. No further monitor events are applied: only
    // the background capacity backstop can deliver the retained stall.
    let occupying = rx.recv().await.expect("occupying note");
    assert!(matches!(
        occupying.notification,
        SubagentNotification::Completed { .. }
    ));
    let delivered = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("backstop must deliver the stall without further child events")
        .expect("channel must stay open");
    assert!(matches!(
        delivered.notification,
        SubagentNotification::Stalled { .. }
    ));
    assert!(
        registry
            .lock()
            .unwrap()
            .get("worker")
            .is_some_and(|entry| entry.pending_stall.is_none()),
        "backstop delivery must clear pending state"
    );
}

// #1082 review Fix 1: a `workflow_idle` with a deliberate reason (explicit
// abort, completion) or with no reason at all must NOT be classified as a
// stall — only `exhausted` is intervention-worthy.
#[tokio::test]
async fn non_exhausted_workflow_idle_reasons_stay_silent() {
    for idle in [
        serde_json::json!({"type":"workflow_idle","reason":"explicit_abort"}),
        serde_json::json!({"type":"workflow_idle","reason":"completed"}),
        serde_json::json!({"type":"workflow_idle"}), // older child, no reason
    ] {
        let registry = new_registry();
        insert_entry(&registry, "worker");
        let (tx, mut rx) = new_notification_channel();
        apply_and_notify(
            &registry,
            Some(&tx),
            "worker",
            &serde_json::json!({"type":"workflow_state","mode":"active","progress":{"done":1,"total":3}}),
        );
        apply_and_notify(&registry, Some(&tx), "worker", &idle);
        assert!(
            rx.try_recv().is_err(),
            "reason {:?} must not raise a stall alert",
            idle.get("reason")
        );
    }
}

// #1082 review Fix 2: a run that ended in a run-level error must produce ONE
// verdict (Errored), not a contradictory Errored + Stalled pair for the same
// run.
#[tokio::test]
async fn errored_run_is_not_also_classified_as_stalled() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_state","mode":"active","progress":{"done":1,"total":3}}),
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"response","command":"agent_error","error":"provider exploded"}),
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_idle","reason":"exhausted"}),
    );
    assert!(
        !std::iter::from_fn(|| rx.try_recv().ok())
            .any(|n| matches!(n.notification, SubagentNotification::Stalled { .. })),
        "an errored run must not additionally raise a stall alert"
    );
}

// #1082 review Fix 2 (re-arm): the run_error guard is scoped to the failed
// run — the next agent_start clears it, so a later genuine stall still alerts.
#[tokio::test]
async fn stall_classification_rearms_after_errored_run_restarts() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"response","command":"agent_error","error":"boom"}),
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"agent_start"}),
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_state","mode":"active","progress":{"done":2,"total":5}}),
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"workflow_idle","reason":"exhausted"}),
    );
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok())
            .any(|n| matches!(n.notification, SubagentNotification::Stalled { .. })),
        "a new run after an errored one must stall-alert normally"
    );
}
