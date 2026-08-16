use super::subagent_registry::{
    NotificationTx, SequencedSubagentNotification, SubagentRegistry, WorkflowSnapshot,
    effective_status,
};

/// Send a stall alert, retaining it as a retryable pending stall on the
/// registry entry when the bounded channel is saturated. Supervision-critical:
/// the alert must survive saturation (#1076 review).
pub(super) fn deliver_or_retain_stall(
    registry: &SubagentRegistry,
    tx: &NotificationTx,
    agent_id: &str,
    notification: SequencedSubagentNotification,
) {
    if let Err(err) = tx.try_send(notification) {
        retain_pending_stall(registry, tx, agent_id, err.into_inner());
    }
}

/// Retain a saturated stall alert for retry, and arm a capacity backstop.
///
/// A stalled child emits no further events, so a retry driven only by that
/// child's own event stream would starve in exactly the case the alert exists
/// for. Two triggers cover it: `retry_pending_stalls` runs on ANY agent's
/// monitor event, and — when a runtime is available — a background task waits
/// for channel capacity directly. `claim_pending_stall` keeps the two paths
/// exactly-once: the notification is owned by whichever claims it first.
fn retain_pending_stall(
    registry: &SubagentRegistry,
    tx: &NotificationTx,
    agent_id: &str,
    notification: SequencedSubagentNotification,
) {
    {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = entries.get_mut(agent_id) else {
            return;
        };
        entry.pending_stall = Some(notification.clone());
    }
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let registry = registry.clone();
        let tx = tx.clone();
        let agent_id = agent_id.to_string();
        handle.spawn(async move {
            // Wait for capacity FIRST, without claiming (#1082 review round 2):
            // while this task waits, a lifecycle event (agent_start,
            // agent_error, workflow progress or completion) can still
            // invalidate the retained stall by clearing `pending_stall`. Only
            // once a send permit is in hand is the notification claimed — a
            // failed claim means it was superseded or already delivered, and
            // the permit is dropped unused. An Err from reserve means the
            // receiver is gone (parent shutting down) and the alert is moot.
            let Ok(permit) = tx.reserve().await else {
                return;
            };
            if claim_pending_stall(&registry, &agent_id, &notification) {
                permit.send(notification);
            }
        });
    }
}

/// Take the pending stall off `agent_id`'s entry iff it is still exactly
/// `expected`. Whoever claims it owns delivery; a lost claim means another
/// path already delivered (or superseded) it.
fn claim_pending_stall(
    registry: &SubagentRegistry,
    agent_id: &str,
    expected: &SequencedSubagentNotification,
) -> bool {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let Some(entry) = entries.get_mut(agent_id) else {
        return false;
    };
    if entry.pending_stall.as_ref() == Some(expected) {
        entry.pending_stall = None;
        true
    } else {
        false
    }
}

/// Retry every retained stall alert, whichever agent it belongs to. The
/// stalled child itself is silent, so any monitor activity elsewhere in the
/// fleet must drive the retry — a same-agent-only retry would starve.
pub(super) fn retry_pending_stalls(
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
) {
    let Some(tx) = notify_tx else { return };
    let pendings: Vec<(String, SequencedSubagentNotification)> = registry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter_map(|(id, entry)| entry.pending_stall.clone().map(|p| (id.clone(), p)))
        .collect();
    for (agent_id, pending) in pendings {
        if !claim_pending_stall(registry, &agent_id, &pending) {
            continue;
        }
        if let Err(err) = tx.try_send(pending) {
            retain_pending_stall(registry, tx, &agent_id, err.into_inner());
        }
    }
}

/// Atomically classify a non-terminal workflow stall, consume its one-shot
/// latch, and capture the payload snapshot. Keeping these decisions under one
/// registry lock prevents a mode change from producing a contradictory alert.
pub(super) fn take_stalled_snapshot(
    registry: &SubagentRegistry,
    agent_id: &str,
) -> Option<WorkflowSnapshot> {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    {
        let entry = entries.get(agent_id)?;
        // #1082 review: a run that terminated with a run-level error has already
        // produced (or will produce) an `Errored` outcome — classifying it as a
        // stall too would deliver contradictory lifecycle verdicts for the same
        // run. `run_error` is cleared on the next `agent_start`, which re-arms
        // stall classification for the new run.
        if entry.run_error.is_some() {
            return None;
        }
        let workflow = entry.workflow.as_ref()?;
        if !entry.stalled_armed
            || !matches!(workflow.mode.as_str(), "active" | "selecting_template")
        {
            return None;
        }
        let own_status = entry.status.clone();
        let effective = effective_status(&entries, agent_id)?;
        if effective.is_active() || (!own_status.is_active() && effective != own_status) {
            return None;
        }
    }
    let entry = entries.get_mut(agent_id)?;
    let snapshot = entry.workflow.as_ref()?.clone();
    entry.stalled_armed = false;
    Some(snapshot)
}

/// Check-and-consume the terminal-completion latch for `agent_id` (#904).
/// Returns `true` (and clears the latch) when a completion note is still armed;
/// `false` when already consumed or the entry is gone. Re-armed by
/// `apply_event_parsed` when the workflow leaves `complete`.
pub(super) fn take_completion_armed(registry: &SubagentRegistry, agent_id: &str) -> bool {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    match entries.get_mut(agent_id) {
        Some(entry) if entry.completion_armed => {
            entry.completion_armed = false;
            true
        }
        _ => false,
    }
}

/// Classify a `workflow_idle` boundary event as a stall, if warranted (#1082
/// review): only an `exhausted` boundary is intervention-worthy — an
/// `explicit_abort` was requested by the parent and a `completed` workflow is
/// handled by the completion path. A missing/unknown reason stays silent
/// (fail-safe: no false alerts from divergent children).
pub(super) fn classify_workflow_idle_stall(
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
    sequence: u64,
    value: &serde_json::Value,
) {
    if value.get("reason").and_then(|v| v.as_str()) != Some("exhausted") {
        return;
    }
    let snapshot = take_stalled_snapshot(registry, agent_id);
    if let (Some(tx), Some(workflow)) = (notify_tx, snapshot) {
        // User-facing stall notes carry the display label (#1378).
        let label = {
            let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
            entries
                .get(agent_id)
                .map(|entry| entry.effective_display_name(agent_id).to_string())
                .unwrap_or_else(|| agent_id.to_string())
        };
        let notification = super::subagent_registry::SequencedSubagentNotification::new(
            sequence,
            super::subagent_registry::SubagentNotification::Stalled {
                agent_id: label,
                workflow_mode: workflow.mode,
                steps_completed: u64::from(workflow.steps_completed),
                steps_total: u64::from(workflow.steps_total),
            },
        );
        deliver_or_retain_stall(registry, tx, agent_id, notification);
    }
}

#[cfg(test)]
#[path = "subagent_monitor_stall_cov_tests.rs"]
mod cov_tests;
