use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::super::subagent_registry::{SubagentEntry, SubagentRegistry};

#[test]
fn register_and_broadcast_assigns_roster_global_sequence() {
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    super::register_and_broadcast(
        &registry,
        None,
        "first",
        SubagentEntry::new(PathBuf::from("/tmp/first.sock"), 1),
    )
    .unwrap();
    let first_sequence = registry
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .notification_sequence;
    assert_eq!(first_sequence, 1);

    super::register_and_broadcast(
        &registry,
        None,
        "second",
        SubagentEntry::new(PathBuf::from("/tmp/second.sock"), 2),
    )
    .unwrap();
    let rows = crate::interface::cli::protocol::build_compact_subagent_roster(
        &Some(registry),
        Some(first_sequence),
    )
    .unwrap();
    assert_eq!(rows.sequence, 2);
    assert_eq!(rows.subagents.len(), 1);
    assert_eq!(rows.subagents[0].agent_id, "second");
}
