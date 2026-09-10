use super::ListEnvironmentsQuery;
use crate::domain::environment_registry::EnvironmentStatus;
use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentRegistry};
use std::path::PathBuf;

fn record(reference: &str) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: reference.to_string(),
        environment_id: format!("runtime-{reference}"),
        environment_uuid: format!("uuid-{reference}"),
        name: None,
        workspace_path: PathBuf::from("/workspace"),
        repository: "repo".into(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    }
}

#[test]
fn empty_registry_returns_empty_detached_snapshot() {
    let registry = EnvironmentRegistry::new();
    let query = ListEnvironmentsQuery::new(registry.clone());
    let snapshot = query.execute();
    assert!(snapshot.is_empty());
    registry.commit(record("C1"));
    assert!(snapshot.is_empty());
}

#[test]
fn preserves_registry_iteration_order_and_complete_records() {
    let registry = EnvironmentRegistry::new();
    let second = record("C2");
    let first = record("C1");
    registry.commit(second.clone());
    registry.commit(first.clone());
    assert_eq!(
        ListEnvironmentsQuery::new(registry).execute(),
        vec![first, second]
    );
}
