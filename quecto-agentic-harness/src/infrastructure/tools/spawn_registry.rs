use super::subagent_registry::{ExitSignal, SubagentEntry, SubagentRegistry};

/// Insert a freshly-spawned child entry into the registry and immediately
/// broadcast the full survivor set, so connected TUIs learn of the new agent at
/// once instead of waiting for the next GetSubagents poll or a terminal event
/// (#866). The broadcast is best-effort: a missing/closed channel just means no
/// client is listening, which is fine.
pub fn register_and_broadcast(
    registry: &SubagentRegistry,
    broadcast_tx: Option<&tokio::sync::broadcast::Sender<String>>,
    session_name: &str,
    entry: SubagentEntry,
) -> Result<(), crate::domain::error::DomainError> {
    // Insert and serialize the survivor set in ONE critical section. Locking
    // twice (insert, then re-lock inside build_state_changed_event) leaves a gap
    // in which a concurrent reaper/cascade-removal could mutate or drop the
    // just-inserted child and broadcast a survivor set that omits it — defeating
    // the immediate-visibility guarantee #866 adds (review).
    let event = {
        let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());
        let mut entry = entry;
        entry.display_name = session_name.to_string();
        guard.insert(entry.agent_uuid.to_string(), entry);
        broadcast_tx.map(|_| {
            crate::infrastructure::tools::subagent_cascade::build_state_changed_event_locked(&guard)
        })
    };
    if let (Some(tx), Some(event)) = (broadcast_tx, event) {
        if let Err(e) = tx.send(event) {
            tracing::debug!(error = %e, "register broadcast had no subscribers");
        }
    }
    Ok(())
}

/// Send SIGTERM to all tracked subagent processes and clear the registry.
/// Also aborts all monitor tasks (#522).
pub fn shutdown_all(registry: &SubagentRegistry) {
    let _ = shutdown_all_with_count(registry);
}

/// Like [`shutdown_all`], returning the number of registry entries removed.
pub fn shutdown_all_with_count(registry: &SubagentRegistry) -> usize {
    // Drain in the SAME critical section that decides who gets signalled, so
    // an agent registering concurrently can never be drained (and cleaned up)
    // without having been signalled, and the returned count matches the
    // drained set exactly.
    let mut removed: Vec<_> = {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        entries.drain().collect()
    };
    for (name, entry) in removed.iter() {
        if let Some(ref tx) = entry.exit_signal_tx {
            let _ = tx.send(Some(ExitSignal {
                exit_code: None,
                signal: Some(15),
            }));
        }
        // Abort monitor task if running (#522).
        if let Some(ref handle) = entry.monitor_handle {
            handle.abort();
            tracing::info!(agent = %name, "aborted monitor task");
        }
        if entry.pid != 0 {
            crate::infrastructure::tools::subagent_cascade::sigterm_pid(entry.pid);
            tracing::info!(agent = %name, pid = entry.pid, "sent SIGTERM to subagent");
        }
    }
    let count = removed.len();
    super::subagent_cleanup::cleanup_removed_entries_sync(&mut removed);
    count
}
