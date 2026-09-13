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

/// #1939: a member's `ProcessIdentity` is never an authority to signal. A
/// live process registered as a member with no endpoint is neither reachable
/// by protocol nor owned by this harness, so the runtime reports the
/// termination failed and the process is untouched. Reintroducing a pid
/// signal on this path kills the sleeper and fails this test.
#[tokio::test]
async fn runtime_never_signals_a_member_by_pid_when_it_is_neither_reachable_nor_owned() {
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
    let error = processes
        .terminate(&member)
        .await
        .expect_err("a member with no endpoint and no owned handle is reported, not signalled");
    assert!(
        error.to_string().contains("not owned by this harness"),
        "{error}"
    );
    // A dead endpoint is equally no authority over the pid.
    let unreachable = Member {
        endpoint: Some(
            _directory
                .path()
                .join("gone.sock")
                .to_string_lossy()
                .into_owned(),
        ),
        ..member.clone()
    };
    let error = processes.terminate(&unreachable).await.unwrap_err();
    assert!(
        error.to_string().contains("did not accept shutdown"),
        "{error}"
    );
    assert!(
        child.try_wait().unwrap().is_none(),
        "the process was never signalled"
    );
    assert!(
        !process_confirmed_dead(identity.pid, &identity.started),
        "the identity is still the observation of a live process"
    );
    child.kill().unwrap();
    child.wait().unwrap();
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
