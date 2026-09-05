use super::*;

// The only PID examined belongs to this test's child. No churn, reassignment,
// or post-reap OS signal is used. A controlled pending reader models an escaped
// pipe holder without creating an uncontained descendant.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancellation_during_pending_drain_keeps_group_leader_owned() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut cmd = build_shell_command(&tmp.path().to_path_buf(), "exit 0", None);
    let child = cmd.spawn().unwrap();
    let pid = child.id().unwrap();
    let stat = format!("/proc/{pid}/stat");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = std::fs::read_to_string(&stat).unwrap();
            if state.split_once(") ").unwrap().1.starts_with('Z') {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let reader = tokio::spawn(std::future::pending::<(String, bool)>());
    let reader_abort = reader.abort_handle();
    let streams = StreamTasks {
        stdout_task: Some(reader),
        stderr_task: None,
    };
    let mut execution = Box::pin(run_child_with_timeout(
        child,
        streams,
        Duration::from_secs(30),
        Arc::new(tmp.path().to_path_buf()),
        None,
    ));
    // Poll through normal child completion into the controlled pending drain.
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut execution)
            .await
            .is_err()
    );
    let owned_during_drain = std::fs::read_to_string(&stat).is_ok();
    drop(execution); // cancellation; any guard must still own its group identity
    reader_abort.abort();
    assert!(
        owned_during_drain,
        "shell was reaped while pending drainage still allowed cancellation signaling"
    );
}
