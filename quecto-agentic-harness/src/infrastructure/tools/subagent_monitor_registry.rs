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
    }
    next_sequence(&mut entries, agent_id).unwrap_or(0)
}

/// Allocate `agent_id`'s next notification sequence under the registry
/// lock: past every sequence any entry holds. `None` without the entry.
pub(super) fn next_sequence(
    entries: &mut std::collections::HashMap<String, SubagentEntry>,
    agent_id: &str,
) -> Option<u64> {
    let next = entries
        .values()
        .map(|entry| entry.notification_sequence)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let entry = entries.get_mut(agent_id)?;
    entry.notification_sequence = next;
    Some(next)
}
