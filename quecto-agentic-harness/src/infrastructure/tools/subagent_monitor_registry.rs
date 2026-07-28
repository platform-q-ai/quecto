use super::subagent_registry::{SubagentEntry, SubagentRegistry};

#[cfg(test)]
pub fn update_entry(
    registry: &SubagentRegistry,
    agent_id: &str,
    f: impl FnOnce(&mut SubagentEntry),
) {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = entries.get_mut(agent_id) {
        f(entry);
    }
}

/// Update an entry and allocate the next monotonic notification sequence.
pub fn update_entry_next_sequence(
    registry: &SubagentRegistry,
    agent_id: &str,
    f: impl FnOnce(&mut SubagentEntry),
) -> u64 {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = entries.get_mut(agent_id) {
        f(entry);
        entry.notification_sequence = entry.notification_sequence.saturating_add(1);
        entry.notification_sequence
    } else {
        0
    }
}
