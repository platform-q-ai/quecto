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

#[tokio::test]
async fn active_workflow_without_continuation_emits_actionable_stall() {
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
        &serde_json::json!({"type":"agent_end","messages":[]}),
    );

    let notification = rx.try_recv().expect("stable active idle must alert");
    let message = notification.to_message();
    assert!(message.contains("idle with workflow still active"));
    assert!(message.contains("1/3"));
    assert!(message.contains("prompt, steer, abort, or kill"));
    assert!(
        rx.try_recv().is_err(),
        "the stable outcome alerts exactly once"
    );
}

#[tokio::test]
async fn selecting_template_without_continuation_emits_stall() {
    let registry = new_registry();
    insert_entry(&registry, "selector");
    let (tx, mut rx) = new_notification_channel();
    apply_and_notify(
        &registry,
        Some(&tx),
        "selector",
        &serde_json::json!({"type":"workflow_state","mode":"selecting_template","progress":{"done":0,"total":0}}),
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "selector",
        &serde_json::json!({"type":"agent_end","messages":[]}),
    );

    let notification = rx.try_recv().expect("stable template selection must alert");
    assert!(notification.to_message().contains("selecting_template"));
    assert!(
        rx.try_recv().is_err(),
        "the stable outcome alerts exactly once"
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
