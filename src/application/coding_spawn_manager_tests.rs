use super::*;

fn test_policy() -> SpawnPolicy {
    SpawnPolicy {
        allow_types: vec![
            "security-reviewer".to_string(),
            "performance-reviewer".to_string(),
            "architecture-reviewer".to_string(),
            "documentation-updater".to_string(),
        ],
        max_depth: 1,
        max_spawns_per_job: 3,
    }
}

fn test_request(id: &str, agent_type: &str) -> SpawnRequest {
    SpawnRequest {
        request_id: id.to_string(),
        agent_type: agent_type.to_string(),
        scope: "current diff".to_string(),
        expected_output: None,
    }
}

#[test]
fn test_approve_allowlisted_type() {
    let mut mgr = SpawnManager::new(test_policy());
    let decision = mgr.evaluate(&test_request("s1", "security-reviewer"));
    assert!(decision.approved);
    assert!(decision.reason.is_none());
    assert_eq!(mgr.spawn_count(), 1);
}

#[test]
fn test_deny_non_allowlisted_type() {
    let mut mgr = SpawnManager::new(test_policy());
    let decision = mgr.evaluate(&test_request("s1", "unknown-agent"));
    assert!(!decision.approved);
    assert!(
        decision
            .reason
            .unwrap()
            .contains("agent type is not allowed")
    );
    assert_eq!(mgr.spawn_count(), 0);
}

#[test]
fn test_deny_when_limit_reached() {
    let mut mgr = SpawnManager::new(test_policy());
    mgr.evaluate(&test_request("s1", "security-reviewer"));
    mgr.evaluate(&test_request("s2", "performance-reviewer"));
    mgr.evaluate(&test_request("s3", "architecture-reviewer"));
    let decision = mgr.evaluate(&test_request("s4", "documentation-updater"));
    assert!(!decision.approved);
    assert!(
        decision
            .reason
            .unwrap()
            .contains("per-job spawn limit is reached")
    );
}

#[test]
fn test_deny_when_max_depth_reached() {
    let mut mgr = SpawnManager::new(test_policy());
    mgr.set_current_depth(1);
    let decision = mgr.evaluate(&test_request("s1", "security-reviewer"));
    assert!(!decision.approved);
    assert!(
        decision
            .reason
            .unwrap()
            .contains("max spawn depth is reached")
    );
}

#[test]
fn test_deduplication_returns_first_request_id() {
    let mut mgr = SpawnManager::new(test_policy());
    let d1 = mgr.evaluate(&test_request("s1", "security-reviewer"));
    assert!(d1.approved);
    assert!(d1.dedup_of.is_none());
    let d2 = mgr.evaluate(&SpawnRequest {
        request_id: "s2".to_string(),
        agent_type: "security-reviewer".to_string(),
        scope: "current diff".to_string(),
        expected_output: None,
    });
    assert!(d2.approved);
    assert_eq!(d2.dedup_of.as_deref(), Some("s1"));
}

#[test]
fn test_record_result_success() {
    let mut mgr = SpawnManager::new(test_policy());
    mgr.evaluate(&test_request("s1", "security-reviewer"));
    let result = mgr.record_result(SpawnResult {
        request_id: "s1".to_string(),
        state: "succeeded".to_string(),
        summary: Some("done".to_string()),
        artifact_refs: vec![],
    });
    assert!(result.is_ok());
    assert!(mgr.is_terminal("s1"));
    assert_eq!(mgr.results().len(), 1);
}

#[test]
fn test_record_result_unknown_request() {
    let mut mgr = SpawnManager::new(test_policy());
    let result = mgr.record_result(SpawnResult {
        request_id: "unknown".to_string(),
        state: "succeeded".to_string(),
        summary: None,
        artifact_refs: vec![],
    });
    assert_eq!(result, Err(SpawnError::UnknownRequestId));
}

#[test]
fn test_cancel_all_returns_active_ids() {
    let mut mgr = SpawnManager::new(test_policy());
    mgr.evaluate(&test_request("s1", "security-reviewer"));
    mgr.evaluate(&test_request("s2", "performance-reviewer"));
    // Mark s1 as terminal
    mgr.record_result(SpawnResult {
        request_id: "s1".to_string(),
        state: "succeeded".to_string(),
        summary: None,
        artifact_refs: vec![],
    })
    .unwrap();
    let canceled = mgr.cancel_all();
    assert_eq!(canceled, vec!["s2"]);
    assert!(mgr.is_terminal("s1"));
    assert!(mgr.is_terminal("s2"));
}

#[test]
fn test_expected_output_forwarded() {
    let mut mgr = SpawnManager::new(test_policy());
    mgr.evaluate(&SpawnRequest {
        request_id: "s1".to_string(),
        agent_type: "security-reviewer".to_string(),
        scope: "current diff".to_string(),
        expected_output: Some("findings.json".to_string()),
    });
    assert_eq!(mgr.expected_output("s1"), Some("findings.json"));
}

#[test]
fn test_is_known_request() {
    let mut mgr = SpawnManager::new(test_policy());
    assert!(!mgr.is_known_request("s1"));
    mgr.evaluate(&test_request("s1", "security-reviewer"));
    assert!(mgr.is_known_request("s1"));
    assert!(!mgr.is_known_request("s2"));
}

#[test]
fn test_spawn_error_display() {
    assert_eq!(
        SpawnError::UnknownRequestId.to_string(),
        "unknown request_id"
    );
    assert_eq!(
        SpawnError::AlreadyTerminal.to_string(),
        "spawn already terminal"
    );
}

#[test]
fn test_double_record_rejected() {
    let mut mgr = SpawnManager::new(test_policy());
    mgr.evaluate(&test_request("s1", "security-reviewer"));
    mgr.record_result(SpawnResult {
        request_id: "s1".to_string(),
        state: "succeeded".to_string(),
        summary: None,
        artifact_refs: vec![],
    })
    .unwrap();
    let result = mgr.record_result(SpawnResult {
        request_id: "s1".to_string(),
        state: "failed".to_string(),
        summary: None,
        artifact_refs: vec![],
    });
    assert_eq!(result, Err(SpawnError::AlreadyTerminal));
}

#[test]
fn test_multiple_concurrent_spawns() {
    let mut mgr = SpawnManager::new(test_policy());
    let d1 = mgr.evaluate(&test_request("s1", "security-reviewer"));
    let d2 = mgr.evaluate(&test_request("s2", "performance-reviewer"));
    let d3 = mgr.evaluate(&test_request("s3", "architecture-reviewer"));
    assert!(d1.approved);
    assert!(d2.approved);
    assert!(d3.approved);
    assert_eq!(mgr.spawn_count(), 3);
}

#[test]
fn test_cancel_all_idempotent() {
    let mut mgr = SpawnManager::new(test_policy());
    mgr.evaluate(&test_request("s1", "security-reviewer"));
    let first = mgr.cancel_all();
    assert_eq!(first.len(), 1);
    let second = mgr.cancel_all();
    assert!(second.is_empty());
}

#[test]
fn test_debug_format() {
    let mgr = SpawnManager::new(test_policy());
    let debug = format!("{:?}", mgr);
    assert!(debug.contains("SpawnManager"));
}
