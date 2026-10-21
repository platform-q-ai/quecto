use super::super::{EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus};

fn record(status: EnvironmentStatus, metadata: serde_json::Value) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: "env-1".into(),
        environment_uuid: "u1".into(),
        name: None,
        workspace_path: "/state/env-1/workspace".into(),
        repository: String::new(),
        script_name: "standard".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec!["a".into()],
        status,
        metadata,
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    }
}

/// #2681: the reason `retain_with` records is the one a retained record
/// reads back; no other status has one, whatever its metadata holds.
#[test]
fn a_retained_record_reads_back_the_reason_it_was_kept_for() {
    let mut kept = record(EnvironmentStatus::Running, serde_json::json!(null));
    kept.retain_with("kept so its work can be pushed");
    assert_eq!(
        kept.retained_reason(),
        Some("kept so its work can be pushed")
    );
    assert!(kept.members.is_empty());
    for status in [
        EnvironmentStatus::Running,
        EnvironmentStatus::Killing,
        EnvironmentStatus::Stopped,
        EnvironmentStatus::CleanupFailed,
    ] {
        let other = record(status.clone(), serde_json::json!({"retained": "old"}));
        assert_eq!(other.retained_reason(), None, "{status:?}");
    }
    for metadata in [
        serde_json::json!({}),
        serde_json::json!({"retained": ""}),
        serde_json::json!({"retained": 7}),
    ] {
        let unnamed = record(EnvironmentStatus::Retained, metadata.clone());
        assert_eq!(unnamed.retained_reason(), None, "{metadata}");
    }
}

/// #2682: the checkout is the create's absolute `metadata.checkout`, else
/// the workspace.
#[test]
fn the_checkout_is_the_advertised_one_else_the_workspace() {
    let advertised = record(
        EnvironmentStatus::Running,
        serde_json::json!({"checkout": "/state/env-1/workspace/repo"}),
    );
    assert_eq!(
        advertised.checkout(),
        std::path::PathBuf::from("/state/env-1/workspace/repo")
    );
    for metadata in [
        serde_json::json!({}),
        serde_json::json!({"checkout": "repo"}),
        serde_json::json!({"checkout": 3}),
    ] {
        let fallback = record(EnvironmentStatus::Running, metadata.clone());
        assert_eq!(
            fallback.checkout(),
            std::path::PathBuf::from("/state/env-1/workspace"),
            "{metadata}"
        );
    }
}
