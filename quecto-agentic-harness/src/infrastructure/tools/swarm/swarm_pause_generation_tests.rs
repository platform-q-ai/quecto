use super::*;
/// #1939: a member's `ProcessIdentity` is never an authority to signal. A
/// live process registered as a member with no endpoint is neither reachable
/// by protocol nor owned by this harness, so the runtime reports the
/// termination failed and the process is untouched. Reintroducing a pid
/// signal on this path kills the sleeper and fails this test.
#[tokio::test]
async fn runtime_never_signals_a_member_by_pid_when_it_is_neither_reachable_nor_owned() {
    use std::os::unix::process::CommandExt;
    let directory = tempfile::tempdir().unwrap();
    let processes = RuntimeProcesses(local_suspend());
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
        launcher: None,
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
            directory
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
        false,
    )
    .unwrap();
    let snapshot = context.snapshot().unwrap();
    assert_eq!(snapshot.status, RunStatus::Paused);
    assert_eq!(snapshot.members.len(), 1);
    assert!(settle_observed_snapshot(&context, &snapshot));
    assert_eq!(context.snapshot().unwrap().status, RunStatus::Paused);
}
