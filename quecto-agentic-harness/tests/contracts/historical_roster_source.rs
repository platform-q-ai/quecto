//! Contract for the `HistoricalRosterSource` port (#1860, #1937, D5
//! #1972): the registry's rows as history — identity, display name,
//! liveness, status and delivery bookkeeping, never a socket or a pid —
//! with their default restore reason; the transaction stamps and orders
//! them.
use std::sync::Arc;

use quecto::application::sessions::ports::HistoricalRosterSource;
use quecto::domain::ids::AgentUuid;
use quecto::domain::session::{SubagentLiveness, SubagentRestoreReason};
use quecto::infrastructure::persistence::session_snapshot_sources::RegistryRosterSource;
use quecto::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentRegistry, new_registry,
};

fn under_test(registry: SubagentRegistry) -> Arc<dyn HistoricalRosterSource> {
    Arc::new(RegistryRosterSource::new(registry))
}

#[test]
fn an_empty_registry_has_no_rows() {
    assert!(under_test(new_registry()).roster_rows().is_empty());
}

#[test]
fn every_entry_becomes_one_history_row_with_the_default_reason() {
    let registry = new_registry();
    {
        let mut entries = registry.lock().unwrap();
        let mut worker = SubagentEntry::with_identity(
            AgentUuid::from("w".to_string()),
            "worker".to_string(),
            "/tmp/w.sock".into(),
            4242,
        );
        worker.persisted_liveness = SubagentLiveness::Detached;
        worker.parent_id = Some("parent".into());
        worker.read_only = true;
        worker.delivered_message_ordinal = Some(3);
        entries.insert("w".to_string(), worker);
    }
    let rows = under_test(registry).roster_rows();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.agent_uuid, "w");
    assert_eq!(row.display_name, "worker");
    assert_eq!(row.session_key, "w");
    assert_eq!(row.liveness, SubagentLiveness::Detached);
    assert_eq!(row.restore_reason, SubagentRestoreReason::LegacyUnspecified);
    assert_eq!(row.parent_id.as_deref(), Some("parent"));
    assert!(row.read_only);
    assert_eq!(row.delivered_message_ordinal, Some(3));
    assert!(row.status.is_some());
}

#[test]
fn rows_carry_no_recovery_authority() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "w".into(),
        SubagentEntry::with_identity(
            AgentUuid::from("w".to_string()),
            "worker".to_string(),
            "/tmp/w.sock".into(),
            4242,
        ),
    );
    let json = serde_json::to_value(under_test(registry).roster_rows()).unwrap();
    let row = &json[0];
    assert!(row.get("pid").is_none(), "pid written: {row}");
    assert!(row.get("socketPath").is_none(), "socket written: {row}");
    assert!(!json.to_string().contains("/tmp/w.sock"));
    assert!(!json.to_string().contains("4242"));
}
