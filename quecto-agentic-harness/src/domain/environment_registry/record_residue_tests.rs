use std::path::PathBuf;

use super::GONE_AT_RESTORE;
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus,
};

fn record(workspace: &str, status: EnvironmentStatus) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: "env-1".into(),
        environment_uuid: "uuid-1".into(),
        name: None,
        workspace_path: workspace.into(),
        repository: String::new(),
        script_name: "official".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: "cli:one".into(),
        created_at: Some(1),
    }
}

#[test]
fn the_environment_dir_is_the_workspace_ancestor_named_after_the_id() {
    let at = |workspace| record(workspace, EnvironmentStatus::Running).environment_dir();
    assert_eq!(at("/s/env-1/workspace"), Some(PathBuf::from("/s/env-1")));
    assert_eq!(
        at("/s/env-1/workspace/repo"),
        Some(PathBuf::from("/s/env-1"))
    );
    assert_eq!(at("/s/env-1"), Some(PathBuf::from("/s/env-1")));
    assert_eq!(at("/w"), None);
    assert_eq!(at("/s/env-10/workspace"), None, "a name is matched whole");
}

#[test]
fn only_a_stopped_record_without_the_older_builds_signature_is_plain_stopped() {
    let stopped = record("/s/env-1/workspace", EnvironmentStatus::Stopped);
    assert!(stopped.is_plain_stopped());
    let mut relabelled = stopped.clone();
    relabelled.metadata = serde_json::json!({"retained": "run r1 unfinished"});
    relabelled.last_error = Some(GONE_AT_RESTORE.into());
    assert!(relabelled.relabelled_while_retained());
    assert!(!relabelled.is_plain_stopped());
    // Either half of the signature alone is a plain stopped record.
    let mut reason_only = relabelled.clone();
    reason_only.last_error = Some(format!("{GONE_AT_RESTORE}; earlier: kill failed"));
    assert!(reason_only.is_plain_stopped());
    let mut error_only = relabelled.clone();
    error_only.metadata = serde_json::json!({});
    assert!(error_only.is_plain_stopped());
    for status in [
        EnvironmentStatus::Running,
        EnvironmentStatus::Killing,
        EnvironmentStatus::CleanupFailed,
        EnvironmentStatus::Retained,
    ] {
        assert!(!record("/s/env-1/workspace", status).is_plain_stopped());
    }
}
