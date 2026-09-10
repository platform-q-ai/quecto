use quecto::domain::swarm::{CoordinationPort, MemberStatus, ProcessIdentity, RunStatus};
use quecto::infrastructure::tools::swarm_bridge::SwarmContext;

#[test]
fn packaged_adapter_enforces_reservations_and_retains_unconfirmed_execution_scopes() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join(".quecto")).unwrap();
    let context = SwarmContext {
        checkout: root.path().to_path_buf(),
        member: "parent".into(),
        lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
    };
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 60;
    context.create_run(&serde_json::json!({"goal":"ship", "constraints":[],"criteria":[{"id":"test","kind":"command","description":"pass"}],"member_limit":2,"deadline":deadline}), &ProcessIdentity { pid: 123, started: "test-identity".into() }, None).unwrap();
    let port: &dyn CoordinationPort = &context;
    port.register_endpoint("/test-endpoint").unwrap();
    port.reserve_member("worker", "token").unwrap();
    port.reserve_member("worker", "token").unwrap();
    assert!(port.reserve_member("overflow", "other").is_err());
    port.confirm_unlaunched("worker").unwrap();
    assert_eq!(
        port.snapshot().unwrap().members[1].status,
        MemberStatus::Dead
    );
    port.reserve_member("replacement", "replacement-token")
        .unwrap();
    port.record_launch(
        "replacement",
        "replacement-token",
        &ProcessIdentity {
            pid: 456,
            started: "worker-identity".into(),
        },
    )
    .unwrap();
    assert!(port.confirm_unlaunched("replacement").is_err());
    port.quarantine("replacement").unwrap();
    let snapshot = port.snapshot().unwrap();
    // A lost harness ends the run as a pause holding `failed` (#1729).
    assert_eq!(snapshot.status, RunStatus::Paused);
    assert_eq!(snapshot.outcome, Some(RunStatus::Failed));
    assert!(snapshot.ended());
    assert_eq!(
        snapshot.members[0].endpoint.as_deref(),
        Some("/test-endpoint")
    );
    assert_eq!(snapshot.members[2].status, MemberStatus::Reserved);
    assert!(port.reserve_member("unsafe", "unsafe-token").is_err());
}
