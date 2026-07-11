use super::subagent_registry::{NotificationTx, SubagentRegistry, WorkflowSnapshot};

/// Retry a stall alert retained after bounded-channel saturation. The pending
/// value is removed only after the exact sequenced notification is accepted.
pub(super) fn retry_pending_stall(
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
) {
    let Some(tx) = notify_tx else { return };
    let pending = registry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(agent_id)
        .and_then(|entry| entry.pending_stall.clone());
    let Some(pending) = pending else { return };
    if tx.try_send(pending.clone()).is_ok() {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = entries.get_mut(agent_id)
            && entry.pending_stall.as_ref() == Some(&pending)
        {
            entry.pending_stall = None;
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
    let entry = entries.get_mut(agent_id)?;
    let workflow = entry.workflow.as_ref()?;
    if !entry.stalled_armed || !matches!(workflow.mode.as_str(), "active" | "selecting_template") {
        return None;
    }
    let snapshot = workflow.clone();
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
