use super::*;
use crate::domain::tool::Tool;
#[tokio::test]
async fn stale_pause_settlement_preserves_resumed_python_job() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = std::sync::Arc::new(directory.path().to_path_buf());
    let tool = super::super::swarm_test_support::tool(
        workspace.clone(),
        std::sync::Arc::new(crate::infrastructure::security::sandbox::Sandbox::new(
            Some(workspace.as_ref().clone()),
        )),
        super::super::swarm::SwarmConfig::default(),
    );
    let context = SwarmContext {
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
    };
    context.pause("old pause").unwrap();
    let old = context.snapshot().unwrap();
    context.resume_external().unwrap();
    let started = tool.execute(r#"{"op":"run","code":"import time; time.sleep(0.1); print('resumed')","background":true}"#).await.unwrap();
    let started: Value = serde_json::from_str(&started.content).unwrap();
    crate::application::swarm::settle(&old, &context.member, &RuntimeProcesses(&context))
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let result = tool
            .execute(&json!({"op":"status","job_id":started["job_id"]}).to_string())
            .await
            .unwrap();
        let value: Value = serde_json::from_str(&result.content).unwrap();
        if matches!(value["status"].as_str(), Some("running" | "cancelling")) {
            assert!(std::time::Instant::now() < deadline);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        } else {
            assert_eq!(value["status"], "completed", "{value}");
            break;
        }
    }
}

#[tokio::test]
async fn runtime_cleanup_checks_process_identity_even_when_abort_endpoint_is_absent() {
    use std::os::unix::process::CommandExt;
    let (_directory, context) = crate::swarm_control_fixture::context();
    let processes = RuntimeProcesses(&context);
    let mut child = std::process::Command::new("/bin/sleep")
        .arg("30")
        .process_group(0)
        .spawn()
        .unwrap();
    let identity = ProcessIdentity {
        pid: child.id(),
        started: process_start(child.id()).unwrap(),
    };
    let member = Member {
        id: "worker".into(),
        status: MemberStatus::Live,
        process: Some(identity.clone()),
        endpoint: None,
    };
    assert!(!processes.abort(&member).await);
    processes
        .terminate(&ProcessIdentity {
            started: "stale-identity".into(),
            ..identity.clone()
        })
        .await
        .unwrap();
    assert!(child.try_wait().unwrap().is_none());
    processes.terminate(&identity).await.unwrap();
    assert!(!child.wait().unwrap().success());
}

#[test]
fn paused_coordinator_reattaches_without_reopening_admission() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    context.pause("approval").unwrap();
    join_current_process(
        &context,
        Some(std::path::Path::new("/test-supervisor.sock")),
        super::super::swarm_bridge::Participation::none(),
    )
    .unwrap();
    let snapshot = context.snapshot().unwrap();
    assert_eq!(snapshot.status, RunStatus::Paused);
    assert_eq!(snapshot.members.len(), 1);
    assert!(settle_observed_snapshot(&context, &snapshot));
    assert_eq!(context.snapshot().unwrap().status, RunStatus::Paused);
}
