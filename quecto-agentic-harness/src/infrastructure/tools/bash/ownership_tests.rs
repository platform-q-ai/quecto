use super::*;

// The only PID examined belongs to this test's child. No churn, reassignment,
// or post-reap OS signal is used. A controlled pending reader models an escaped
// pipe holder without creating an uncontained descendant.
#[cfg(unix)]
#[tokio::test]
async fn cancellation_during_pending_drain_keeps_group_leader_owned() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut cmd = build_shell_command(&tmp.path().to_path_buf(), "exit 0", None);
    let mut child = cmd.spawn().unwrap();
    let pid = child.id().unwrap();
    tokio::time::timeout(Duration::from_secs(5), wait_owned::exited(&mut child))
        .await
        .unwrap()
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
    // A second WNOWAIT observation succeeds only while the exited child is
    // still ours to reap. Unlike /proc, this also runs on non-Linux Unix.
    // SAFETY: zero is a valid initial siginfo_t representation for waitid.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: writable siginfo; observation only, never signal a saved PID.
    let rc = unsafe {
        libc::waitid(
            libc::P_PID,
            pid,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    // SAFETY: short-circuiting reads the child field only after successful waitid.
    let owned_during_drain = rc == 0 && unsafe { info.si_pid() } == pid as libc::pid_t;
    drop(execution); // cancellation; any guard must still own its group identity
    reader_abort.abort();
    assert!(
        owned_during_drain,
        "shell was reaped while pending drainage still allowed cancellation signaling"
    );
}

// A real same-group survivor holds stdout after the shell exits. Cancellation
// must kill it while the unreaped shell still pins the PGID. The bounded sleep
// also ensures a failing regression cannot leave a permanent orphan behind.
#[cfg(unix)]
#[tokio::test]
async fn cancellation_during_drain_kills_same_group_pipe_holder() {
    use tokio::io::AsyncReadExt;

    let tmp = tempfile::TempDir::new().unwrap();
    let mut cmd = build_shell_command(&tmp.path().to_path_buf(), "sleep 5 & exit 0", None);
    cmd.stdout(std::process::Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    tokio::time::timeout(Duration::from_secs(5), wait_owned::exited(&mut child))
        .await
        .unwrap()
        .unwrap();
    let (eof_tx, mut eof_rx) = tokio::sync::oneshot::channel();
    let reader = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.unwrap();
        let _ = eof_tx.send(());
        (String::new(), false)
    });
    let mut execution = Box::pin(run_child_with_timeout(
        child,
        StreamTasks {
            stdout_task: Some(reader),
            stderr_task: None,
        },
        Duration::from_secs(30),
        Arc::new(tmp.path().to_path_buf()),
        None,
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut execution)
            .await
            .is_err()
    );
    assert!(matches!(
        eof_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    drop(execution);
    tokio::time::timeout(Duration::from_secs(2), eof_rx)
        .await
        .expect("same-group survivor kept stdout open after cancellation")
        .unwrap();
}
