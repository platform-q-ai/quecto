use std::collections::hash_map::Entry;

use super::harness_lifecycle::{SharedHarnessLifecycle, admit_spawn};
use super::subagent_registry::{SubagentEntry, SubagentRegistry};

/// Insert a freshly-spawned child entry into the registry and immediately
/// broadcast the full survivor set, so connected TUIs learn of the new agent at
/// once instead of waiting for the next GetSubagents poll or a terminal event
/// (#866). The broadcast is best-effort: a missing/closed channel just means no
/// client is listening, which is fine.
///
/// Admission against the harness lifecycle (#1938) is decided **inside the
/// registry critical section**: a shutdown freezes the lifecycle and then
/// claims the direct children under this same lock, so a registration is
/// either inserted before that claim — and torn down with the fleet — or
/// observes the freeze and is refused. Never both, never neither.
pub fn register_and_broadcast(
    registry: &SubagentRegistry,
    broadcast_tx: Option<&tokio::sync::broadcast::Sender<String>>,
    session_name: &str,
    entry: SubagentEntry,
    lifecycle: &SharedHarnessLifecycle,
) -> Result<(), crate::domain::error::DomainError> {
    // Insert and serialize the survivor set in ONE critical section. Locking
    // twice (insert, then re-lock inside build_state_changed_event) leaves a gap
    // in which a concurrent reaper/cascade-removal could mutate or drop the
    // just-inserted child and broadcast a survivor set that omits it — defeating
    // the immediate-visibility guarantee #866 adds (review).
    let event = {
        let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());
        admit_spawn(lifecycle).map_err(|refused| {
            crate::domain::error::DomainError::Other(format!(
                "{refused} while registering {session_name}"
            ))
        })?;
        let mut entry = entry;
        entry.display_name = session_name.to_string();
        entry.notification_sequence = guard
            .values()
            .map(|existing| existing.notification_sequence)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        match guard.entry(entry.agent_uuid.to_string()) {
            Entry::Vacant(slot) => {
                slot.insert(entry);
            }
            Entry::Occupied(existing) => {
                return Err(crate::domain::error::DomainError::Other(format!(
                    "duplicate subagent registry key {} while registering {session_name}",
                    existing.key()
                )));
            }
        }
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
