//! Slice 3 (#1369): post-mortem inspect on the authoritative environment
//! aggregate. Exactly one inspect claim per dead member, aggregate updated
//! BEFORE member removal, outcome survives zero members, failures persisted
//! truthfully with retained context for retry.

use super::*;

fn record(env_ref: &str) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: env_ref.to_string(),
        environment_id: format!("id-{env_ref}"),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: std::path::PathBuf::from("/tmp/ws"),
        repository: "https://example.invalid/repo.git".into(),
        script_name: "default".into(),
        retained_exec_argv: vec!["exec".into()],
        retained_kill_argv: vec!["kill".into()],
        retained_cleanup_argv: vec!["cleanup".into()],
        retained_inspect_argv: vec!["inspect".into()],
        members: Vec::new(),
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({"created": true}),
        last_error: None,
    }
}

fn registry_with_member(env_ref: &str, member: &str) -> EnvironmentRegistry {
    let registry = EnvironmentRegistry::new();
    let minted = registry.mint_ref();
    assert_eq!(minted, env_ref);
    registry.commit(record(env_ref));
    registry.add_member(env_ref, member).unwrap();
    registry
}

#[test]
fn record_retains_inspect_argv() {
    let registry = registry_with_member("C1", "a1");
    let rec = registry.get("C1").unwrap();
    assert_eq!(rec.retained_inspect_argv, vec!["inspect".to_string()]);
}

#[test]
fn begin_inspect_is_exactly_once_per_dead_member() {
    let registry = registry_with_member("C1", "a1");
    let claim = registry.begin_inspect("C1", "a1");
    assert!(claim.is_some(), "first death claims the inspect");
    assert!(
        registry.begin_inspect("C1", "a1").is_none(),
        "repeated EOF/reset for the same member must not claim a second inspect"
    );
}

#[test]
fn inspect_success_updates_aggregate_before_member_removal() {
    let registry = registry_with_member("C1", "a1");
    let claim = registry.begin_inspect("C1", "a1").unwrap();
    registry.record_inspect_success(claim, serde_json::json!({"cause": "oom-killed"}));
    // The aggregate is updated while the member is STILL present.
    let rec = registry.get("C1").unwrap();
    assert_eq!(rec.members, vec!["a1".to_string()]);
    assert_eq!(rec.metadata["cause"], serde_json::json!("oom-killed"));
    // Member removal afterwards keeps the inspect outcome.
    registry.remove_member("C1", "a1").unwrap();
    let rec = registry.get("C1").unwrap();
    assert_eq!(rec.metadata["cause"], serde_json::json!("oom-killed"));
}

#[test]
fn inspect_success_merges_metadata_without_discarding_create_metadata() {
    let registry = registry_with_member("C1", "a1");
    let claim = registry.begin_inspect("C1", "a1").unwrap();
    registry.record_inspect_success(claim, serde_json::json!({"cause": "exit"}));
    let rec = registry.get("C1").unwrap();
    assert_eq!(rec.metadata["created"], serde_json::json!(true));
    assert_eq!(rec.metadata["cause"], serde_json::json!("exit"));
}

#[test]
fn inspect_failure_persists_truthfully_with_retained_context() {
    let registry = registry_with_member("C1", "a1");
    let claim = registry.begin_inspect("C1", "a1").unwrap();
    registry.record_inspect_failure(claim, "inspect exited with status 1: boom");
    let rec = registry.get("C1").unwrap();
    let err = rec.last_error.as_deref().unwrap_or_default();
    assert!(err.contains("inspect"), "last_error: {err}");
    assert_eq!(
        rec.retained_inspect_argv,
        vec!["inspect".to_string()],
        "retained inspect argv must survive failure so it can be retried"
    );
}

#[test]
fn inspect_outcome_survives_zero_members() {
    let registry = registry_with_member("C1", "a1");
    let claim = registry.begin_inspect("C1", "a1").unwrap();
    // Death path empties the environment before the inspect outcome lands.
    registry.remove_member("C1", "a1").unwrap();
    registry.record_inspect_success(claim, serde_json::json!({"cause": "oom-killed"}));
    let rec = registry.get("C1").unwrap();
    assert!(rec.members.is_empty());
    assert_eq!(rec.metadata["cause"], serde_json::json!("oom-killed"));
}

#[test]
fn inspect_on_removed_environment_is_a_noop_not_a_panic() {
    let registry = registry_with_member("C1", "a1");
    let claim = registry.begin_inspect("C1", "a1").unwrap();
    registry.remove("C1");
    registry.record_inspect_failure(claim, "inspect failed after removal");
    assert!(registry.get("C1").is_none());
}
