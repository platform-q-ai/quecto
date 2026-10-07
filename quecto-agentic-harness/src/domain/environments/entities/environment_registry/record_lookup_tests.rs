use super::super::{EnvironmentRecord, EnvironmentRegistry};
use super::{EnvironmentStatus, EnvironmentTarget};

fn record(status: EnvironmentStatus) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: "env-1".into(),
        environment_uuid: "uuid-1".into(),
        name: None,
        workspace_path: "/ws".into(),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: Default::default(),
        created_by: String::new(),
        created_at: None,
    }
}

/// #2220: the listing's "joinable" is exactly what a join admits.
#[test]
fn is_joinable_agrees_with_what_a_join_resolves_for_every_status() {
    for (status, joinable) in [
        (EnvironmentStatus::Running, true),
        (EnvironmentStatus::Retained, true),
        (EnvironmentStatus::Killing, false),
        (EnvironmentStatus::Stopped, false),
        (EnvironmentStatus::CleanupFailed, false),
    ] {
        assert_eq!(status.is_joinable(), joinable, "{status:?}");
        let registry = EnvironmentRegistry::new();
        registry.commit(record(status.clone()));
        let resolved = registry.resolve_joinable(&EnvironmentTarget::Ref("C1".into()));
        assert_eq!(resolved.is_ok(), joinable, "{status:?}: {resolved:?}");
    }
}
